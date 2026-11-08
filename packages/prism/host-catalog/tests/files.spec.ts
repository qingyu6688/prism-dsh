/** 读文件、上传、清理上传目录的测试，用临时目录当数据目录。 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, readdir, rm, utimes, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { FILE_PATH, UPLOAD_PATH, cleanInbox, receiveUpload, resolveInside, safeUploadName, serveFile } from '../src/files.ts'
import type { UploadedFile } from '../src/types.ts'

let dataDir: string

beforeEach(async () => {
  dataDir = await mkdtemp(join(tmpdir(), 'prism-files-'))
})

afterEach(async () => {
  await rm(dataDir, { recursive: true, force: true })
})

function fileRequest(path: string, method = 'GET'): Request {
  return new Request(`http://localhost${FILE_PATH}?path=${encodeURIComponent(path)}`, { method })
}

function uploadRequest(name: string, body: string | Uint8Array, headers: Record<string, string> = {}): Request {
  return new Request(`http://localhost${UPLOAD_PATH}?name=${encodeURIComponent(name)}`, { method: 'POST', body: typeof body === 'string' ? body : new Uint8Array(body), headers })
}

describe('resolveInside', () => {
  it('只接受数据目录里的相对路径', () => {
    expect(resolveInside(dataDir, 'assets/ab/c.png')).toBe(join(dataDir, 'assets', 'ab', 'c.png'))
    for (const bad of ['', '../x.png', 'a/../../x.png', 'a\\..\\..\\x.png', '/etc/x.png', 'C:\\x.png', 'c:x.png', '.']) {
      expect(resolveInside(dataDir, bad), bad).toBeUndefined()
    }
  })
})

describe('serveFile', () => {
  it('返回图片内容和类型；HEAD 只返回头', async () => {
    await mkdir(join(dataDir, 'thumbs'))
    await writeFile(join(dataDir, 'thumbs', '缩略图.webp'), 'webp-bytes')
    const response = await serveFile(dataDir, fileRequest('thumbs/缩略图.webp'))
    expect(response.status).toBe(200)
    expect(response.headers.get('content-type')).toBe('image/webp')
    expect(response.headers.get('content-length')).toBe('10')
    expect(await response.text()).toBe('webp-bytes')

    const head = await serveFile(dataDir, fileRequest('thumbs/缩略图.webp', 'HEAD'))
    expect(head.status).toBe(200)
    expect(await head.text()).toBe('')
  })

  it('越界、非图片、不存在、服务没运行', async () => {
    await writeFile(join(dataDir, 'prism.db'), 'db')
    expect((await serveFile(dataDir, fileRequest('../secret.png'))).status).toBe(400)
    expect((await serveFile(dataDir, fileRequest('prism.db'))).status).toBe(400)
    expect((await serveFile(dataDir, fileRequest('missing.png'))).status).toBe(404)
    await mkdir(join(dataDir, 'dir.png'))
    expect((await serveFile(dataDir, fileRequest('dir.png'))).status).toBe(404)
    const unavailable = await serveFile(undefined, fileRequest('a.png'))
    expect(unavailable.status).toBe(503)
    expect(await unavailable.json()).toMatchObject({ code: 'unavailable' })
  })
})

describe('safeUploadName', () => {
  it('只留文件名，去掉不允许的字符，限制扩展名', () => {
    expect(safeUploadName('商品.xlsx')).toBe('商品.xlsx')
    expect(safeUploadName('C:\\Users\\me\\主图.PNG')).toBe('主图.PNG')
    expect(safeUploadName('../../a.csv')).toBe('a.csv')
    expect(safeUploadName('a:b?.json')).toBe('a_b_.json')
    expect(safeUploadName('run.exe')).toBeUndefined()
    expect(safeUploadName('')).toBeUndefined()
    expect(safeUploadName(`${'长'.repeat(201)}.csv`)).toBeUndefined()
  })
})

describe('receiveUpload', () => {
  it('写到 inbox 下的独立目录，返回 Host 上的路径', async () => {
    const response = await receiveUpload(dataDir, uploadRequest('商品.csv', '货号,名称\n'))
    expect(response.status).toBe(200)
    const uploaded = await response.json() as UploadedFile
    expect(uploaded.name).toBe('商品.csv')
    expect(uploaded.size).toBe(Buffer.byteLength('货号,名称\n'))
    expect(uploaded.path.startsWith(join(dataDir, 'inbox'))).toBe(true)
    expect(await readFile(uploaded.path, 'utf8')).toBe('货号,名称\n')

    // 同名文件再传一次不会覆盖
    const second = await (await receiveUpload(dataDir, uploadRequest('商品.csv', 'x'))).json() as UploadedFile
    expect(second.path).not.toBe(uploaded.path)
  })

  it('超过上限：按声明的长度直接拒绝，或写到一半发现超了就删掉', async () => {
    const declared = await receiveUpload(dataDir, uploadRequest('a.png', 'x'.repeat(10), { 'content-length': '999' }), 5)
    expect(declared.status).toBe(413)
    const streamed = await receiveUpload(dataDir, uploadRequest('a.png', 'x'.repeat(10)), 5)
    expect(streamed.status).toBe(413)
    expect(await readdir(join(dataDir, 'inbox'))).toEqual([])
  })

  it('扩展名不允许、服务没运行', async () => {
    expect((await receiveUpload(dataDir, uploadRequest('a.exe', 'x'))).status).toBe(400)
    expect((await receiveUpload(undefined, uploadRequest('a.csv', 'x'))).status).toBe(503)
  })
})

describe('cleanInbox', () => {
  it('删掉超过时限的上传目录，留下新的', async () => {
    const old = join(dataDir, 'inbox', 'old')
    const fresh = join(dataDir, 'inbox', 'fresh')
    await mkdir(old, { recursive: true })
    await mkdir(fresh, { recursive: true })
    const now = Date.now()
    const twoDaysAgo = new Date(now - 2 * 24 * 60 * 60 * 1000)
    await utimes(old, twoDaysAgo, twoDaysAgo)
    expect(await cleanInbox(dataDir, now)).toBe(1)
    expect(existsSync(old)).toBe(false)
    expect(existsSync(fresh)).toBe(true)
  })

  it('没有 inbox 目录时什么都不做', async () => {
    expect(await cleanInbox(dataDir, Date.now())).toBe(0)
  })
})
