/**
 * 界面读写数据目录文件的两个路由，挂在 dsh Connection 的 `/api` 下（登录检查由 Connection 做，只有本机用户能访问）：
 *
 * - `GET /api/prism/file?path=<相对路径>`：读数据目录里的图片（素材原图、缩略图）
 * - `POST /api/prism/upload?name=<文件名>`：把要导入的表格、图片、规则文件传到数据目录的 `inbox/`，返回 Host 上的路径
 *
 * 浏览器拿不到本地文件的路径，所以导入前先上传；`inbox/` 里超过一天的文件在插件启动时清掉。
 * @module @prism/dsh-host-catalog/files
 */

import { randomUUID } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { mkdir, open, readdir, rm, stat } from 'node:fs/promises'
import { extname, isAbsolute, join, relative, resolve } from 'node:path'
import { Readable } from 'node:stream'
import type { UploadedFile } from './types.ts'

export const FILE_PATH = '/api/prism/file'
export const UPLOAD_PATH = '/api/prism/upload'

/** 上传文件放在数据目录的这个子目录下。 */
export const INBOX_DIR = 'inbox'
/** 单个上传文件的上限。原图上限是 20 MB，表格上限 20 MB，留些余量。 */
export const MAX_UPLOAD_BYTES = 50 * 1024 * 1024
/** 上传后多久没用就清掉。 */
export const INBOX_MAX_AGE_MS = 24 * 60 * 60 * 1000
const MAX_NAME_LENGTH = 200

/** 能读的图片类型。 */
const IMAGE_TYPES: Readonly<Record<string, string>> = {
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
  '.gif': 'image/gif',
}

/** 能上传的扩展名：表格、图片、规则文件。 */
const UPLOAD_EXTENSIONS = new Set([
  '.xlsx', '.xls', '.ods', '.csv',
  '.png', '.jpg', '.jpeg', '.webp',
  '.json', '.yaml', '.yml',
])

function failure(status: number, code: string, message: string): Response {
  return Response.json({ code, message }, { status })
}

/**
 * 相对路径是否安全：不是绝对路径，没有 `..`，解析后仍在数据目录里。
 * @param dataDir - 数据目录。
 * @param relativePath - 界面传来的相对路径。
 * @returns 完整路径；不安全时返回 `undefined`。
 */
export function resolveInside(dataDir: string, relativePath: string): string | undefined {
  if (relativePath === '' || isAbsolute(relativePath) || /^[a-zA-Z]:/.test(relativePath)) return undefined
  if (relativePath.split(/[\\/]/).includes('..')) return undefined
  const root = resolve(dataDir)
  const full = resolve(root, relativePath)
  const rest = relative(root, full)
  if (rest === '' || rest.startsWith('..') || isAbsolute(rest)) return undefined
  return full
}

/**
 * 读数据目录里的一张图片。
 * @param dataDir - 数据目录；服务没起来时为 `undefined`。
 * @param request - GET 或 HEAD 请求，`path` 参数是相对数据目录的路径。
 * @returns 图片内容，或 400 / 404 / 503。
 */
export async function serveFile(dataDir: string | undefined, request: Request): Promise<Response> {
  if (dataDir === undefined) return failure(503, 'unavailable', '棱镜业务服务没有运行')
  const relativePath = new URL(request.url).searchParams.get('path') ?? ''
  const full = resolveInside(dataDir, relativePath)
  const type = IMAGE_TYPES[extname(relativePath).toLowerCase()]
  if (full === undefined || type === undefined) return failure(400, 'invalid_input', '只能读取数据目录里的图片')
  let size: number
  try {
    const info = await stat(full)
    if (!info.isFile()) return failure(404, 'not_found', '文件不存在')
    size = info.size
  } catch {
    // 文件被删了或路径不存在：对界面来说都是"没有这张图"
    return failure(404, 'not_found', '文件不存在')
  }
  const headers = {
    'content-type': type,
    'content-length': String(size),
    // 素材文件按内容哈希命名，内容不会变
    'cache-control': 'private, max-age=86400',
  }
  if (request.method === 'HEAD') return new Response(null, { headers })
  const body = Readable.toWeb(createReadStream(full)) as ReadableStream<Uint8Array>
  return new Response(body, { headers })
}

/**
 * 整理上传的文件名：只留文件名部分，去掉 Windows 不允许的字符。
 * @param name - 界面传来的文件名。
 * @returns 整理后的文件名；为空或扩展名不允许时返回 `undefined`。
 */
export function safeUploadName(name: string): string | undefined {
  // 自己按分隔符取最后一段：path.basename 在 Windows 上会把 `a:` 当成盘符去掉
  const last = name.replaceAll('\\', '/').split('/').pop() ?? ''
  const cleaned = last.replace(/[<>:"|?*\u0000-\u001f]/g, '_').trim()
  if (cleaned === '' || cleaned === '.' || cleaned === '..' || cleaned.length > MAX_NAME_LENGTH) return undefined
  return UPLOAD_EXTENSIONS.has(extname(cleaned).toLowerCase()) ? cleaned : undefined
}

/**
 * 把请求体写到 `inbox/<随机目录>/<文件名>`。
 * @param dataDir - 数据目录；服务没起来时为 `undefined`。
 * @param request - POST 请求，`name` 参数是原文件名。
 * @param maxBytes - 单个文件上限（测试时调小）。
 * @returns `UploadedFile`，或 400 / 413 / 503。
 */
export async function receiveUpload(
  dataDir: string | undefined,
  request: Request,
  maxBytes: number = MAX_UPLOAD_BYTES,
): Promise<Response> {
  if (dataDir === undefined) return failure(503, 'unavailable', '棱镜业务服务没有运行')
  const name = safeUploadName(new URL(request.url).searchParams.get('name') ?? '')
  if (name === undefined) {
    return failure(400, 'invalid_input', '只能上传表格（xlsx、xls、ods、csv）、图片（png、jpg、webp）或规则文件（json、yaml）')
  }
  const tooLarge = (): Response => failure(413, 'too_large', `文件超过 ${Math.round(maxBytes / 1024 / 1024)} MB`)
  if (Number(request.headers.get('content-length') ?? 0) > maxBytes) return tooLarge()

  const dir = join(dataDir, INBOX_DIR, randomUUID())
  const path = join(dir, name)
  await mkdir(dir, { recursive: true })
  const file = await open(path, 'wx')
  let size = 0
  let exceeded = false
  try {
    if (request.body !== null) {
      for await (const chunk of request.body as unknown as AsyncIterable<Uint8Array>) {
        size += chunk.byteLength
        if (size > maxBytes) {
          exceeded = true
          break
        }
        await file.write(chunk)
      }
    }
  } finally {
    await file.close()
  }
  if (exceeded) {
    await rm(dir, { recursive: true, force: true })
    return tooLarge()
  }
  const uploaded: UploadedFile = { path, name, size }
  return Response.json(uploaded)
}

/**
 * 清掉 `inbox/` 里放了太久的上传目录。
 * @param dataDir - 数据目录。
 * @param now - 当前时间（毫秒）。
 * @param maxAgeMs - 超过这个时间的删掉。
 * @returns 删掉的目录数。
 */
export async function cleanInbox(dataDir: string, now: number, maxAgeMs: number = INBOX_MAX_AGE_MS): Promise<number> {
  const inbox = join(dataDir, INBOX_DIR)
  let entries: string[]
  try {
    entries = await readdir(inbox)
  } catch {
    // 还没上传过文件，没有 inbox 目录
    return 0
  }
  let removed = 0
  for (const entry of entries) {
    const path = join(inbox, entry)
    const info = await stat(path)
    if (now - info.mtimeMs > maxAgeMs) {
      await rm(path, { recursive: true, force: true })
      removed++
    }
  }
  return removed
}
