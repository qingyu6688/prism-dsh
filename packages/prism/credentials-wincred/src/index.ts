/** 系统凭据提供器。通过本机 Rust 服务访问系统凭据管理器，读写失败时停止对应操作。 */
import { Service } from '@deepseek-ai/cordis'
import { CredentialProvider, parseCredentialKey } from '@deepseek-ai/dsh-credentials'
import type { CredentialRef, CredentialKey, CredentialRecord, CredentialRecordEntry, CredentialRecordInfo, CredentialInfo, ResolvedCredential } from '@deepseek-ai/dsh-credentials'
import type {} from '@prism/dsh-host-service'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { parseDocument } from 'yaml'
import { parseCredentialsDocument } from '@deepseek-ai/dsh-credentials-local'
import { resolveDshHome } from '@deepseek-ai/dsh-home-paths'
import { withFileLock, writeFileAtomic } from '@deepseek-ai/dsh-atomic-write'
import { launchEnvironmentOf } from '@deepseek-ai/dsh-launch-environment'

/**
   * 从系统存储解析凭据记录；损坏的数据不能当成未配置继续使用。
   * @param text - 已保存的 JSON。
   * @returns 验证后的凭据记录。
   */
export function decodeRecord(text: string): CredentialRecord {
  try {
    const parsed: unknown = JSON.parse(text)
    const record = parseCredentialsDocument(JSON.stringify({ version: 1, records: { 'prism/stored': parsed } }), '系统凭据').records.get('prism/stored')
    if (record === undefined) throw new Error('缺少记录')
    return record
  } catch (error) {
    // 解析器可能带上源值，系统凭据错误只报告修复办法。
    void error
    throw new Error('系统凭据记录损坏，请重新保存')
  }
}

/** 文本模型与生图共用系统凭据存储；记录修改按提供器实例串行。 */
export class WinCredentials extends CredentialProvider {
  static inject = ['prismService']
  private closed = false
  private queue: Promise<void> = Promise.resolve()
  protected async* [Service.init](): AsyncGenerator<() => Promise<void>, void, void> {
    yield async () => { this.closed = true; await this.queue }
    // 只有所有记录写入系统存储成功，才清除旧文件的秘密字段；失败保留原文件。
    const filename = join(resolveDshHome(), '.credentials.yaml')
    let source: string
    try { source = await readFile(filename, 'utf8') }
    catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return; throw error }
    await withFileLock(filename, async () => {
      source = await readFile(filename, 'utf8')
      const document = parseDocument(source)
      if (document.errors.length > 0) throw new Error('旧凭据文件格式无效，尚未迁移，请修复后重启')
      let parsed: ReturnType<typeof parseCredentialsDocument>
      try { parsed = parseCredentialsDocument(source, filename) }
      catch (error) {
        // 不转发可能引用秘密字段的解析错误；原文件保持不变。
        void error
        throw new Error('旧凭据文件格式无效，尚未迁移，请检查版本与记录字段')
      }
      for (const [name, value] of parsed.refs) {
        const key = `ref/${name}`
        if (await this.get(key) === undefined) await this.ctx.prismService.call('credentials.set', { key, value })
      }
      for (const [name, value] of parsed.records) {
        const key = `record/${name}`
        if (await this.get(key) === undefined) await this.ctx.prismService.call('credentials.set', { key, value: JSON.stringify(value) })
      }
      document.delete('refs')
      document.delete('records')
      await writeFileAtomic(filename, document.toString(), { mode: 0o600, dirMode: 0o700 })
    })
  }
  private async get(key: string): Promise<string | undefined> {
    return (await this.ctx.prismService.call<string | null>('credentials.get', { key })) ?? undefined
  }
  /**
   * @param ref - 环境式引用。
   * @returns 系统存储优先，其次进程环境。
   */
  async resolve(ref: CredentialRef): Promise<ResolvedCredential | undefined> {
    const value = await this.get(`ref/${ref}`)
    if (value) return { value, source: 'wincred' }
    const inherited = launchEnvironmentOf(this.ctx).getFrom(ref, ['process', 'project-env', 'user-env'])
    return inherited?.value ? { value: inherited.value, source: inherited.source } : undefined
  }
  /**
   * @param ref - 引用名称。
   * @returns 不包含秘密的状态。
   */
  async describe(ref: CredentialRef): Promise<CredentialInfo> {
    const value = await this.resolve(ref)
    return { configured: value !== undefined, writable: true, ...(value ? { source: value.source } : {}) }
  }
  /**
   * @param ref - 引用名称。
   * @param value - 新值。
   */
  async set(ref: CredentialRef, value: string): Promise<void> {
    await this.enqueue(async () => {
      await this.ctx.prismService.call('credentials.set', { key: `ref/${ref}`, value })
      this.notifyUpdated(ref)
    })
  }
  /**
   * @param ref - 删除的引用。
   */
  async unset(ref: CredentialRef): Promise<void> {
    await this.enqueue(async () => {
      await this.ctx.prismService.call('credentials.delete', { key: `ref/${ref}` })
      this.notifyUpdated(ref)
    })
  }
  /**
   * @param key - 记录地址。
   * @returns 已保存记录，缺少时返回 undefined。
   */
  async readRecord(key: CredentialKey): Promise<CredentialRecord | undefined> {
    const value = await this.get(`record/${key}`)
    return value === undefined ? undefined : decodeRecord(value)
  }
  /**
   * @param key - 记录地址。
   * @returns 记录类型及存在状态。
   */
  async describeRecord(key: CredentialKey): Promise<CredentialRecordInfo> {
    const value = await this.readRecord(key)
    return { configured: value !== undefined, writable: true, ...(value ? { kind: value.kind } : {}) }
  }
  /** @returns 当前存储的记录地址和类型，不含秘密。 */
  async listRecords(): Promise<readonly CredentialRecordEntry[]> {
    const keys = await this.ctx.prismService.call<string[]>('credentials.list')
    const result: CredentialRecordEntry[] = []
    for (const raw of keys.filter(key => key.startsWith('record/'))) {
      const key = parseCredentialKey(raw.slice(7))
      const value = await this.readRecord(key)
      if (value) result.push({ key, kind: value.kind })
    }
    return result
  }
  private enqueue<T>(operation: () => Promise<T>): Promise<T> {
    if (this.closed) return Promise.reject(new Error('凭据服务已关闭'))
    const pending = this.queue.then(operation)
    this.queue = pending.then(() => undefined, () => undefined)
    return pending
  }
  /**
   * @param key - 记录地址。
   * @param mutate - 串行执行的变更。
   * @returns 保存后的记录。
   */
  async modifyRecord(
    key: CredentialKey,
    mutate: (current: CredentialRecord | undefined) => Promise<CredentialRecord | undefined>,
  ): Promise<CredentialRecord | undefined> {
    return await this.enqueue(async () => {
      const current = await this.readRecord(key)
      const value = await mutate(current)
      if (value !== undefined) {
        await this.ctx.prismService.call('credentials.set', { key: `record/${key}`, value: JSON.stringify(value) })
        this.notifyRecordUpdated(key)
      }
      return value ?? current
    })
  }
  /**
   * @param key - 删除的记录地址。
   */
  async deleteRecord(key: CredentialKey): Promise<void> {
    await this.enqueue(async () => {
      await this.ctx.prismService.call('credentials.delete', { key: `record/${key}` })
      this.notifyRecordUpdated(key)
    })
  }
}
export default WinCredentials
