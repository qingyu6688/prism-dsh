// @vitest-environment jsdom
/** 生图配置和结果的用户交互；API 替身不接触真实密钥。 */
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { makeTranslate } from '@deepseek-ai/dsh-client-test-runtime'
import type { ImageBatch, ImageProfile } from '@prism/dsh-host-catalog/types'
import { ImageResults, ImageSettings } from '../src/client/Images.tsx'
import { imageBatchId } from '../src/client/ImageToolCard.tsx'
import type { ImageApi } from '../src/client/image-api.ts'
import { asset } from './fake-api.client.ts'
import { fileUrl } from '../src/client/api.ts'
import { zh } from '../src/client/locales.ts'
import type { Translate } from '../src/client/ProductForm.tsx'

const t = makeTranslate(zh) as Translate
const profile: ImageProfile = { id: 'p1', name: '测试接口', baseUrl: 'https://example.com/v1', model: 'image-test', providerType: 'openai_images', maxConcurrency: 3, timeoutSecs: 600, enabled: true, disabledReason: null, hasKey: true }
const batch: ImageBatch = { seq: 1, id: 'b1', owner: 's1', status: 'running', total: 40, succeeded: 2, failed: 0, assets: [], errors: [], reason: null }
function api() {
  return { profiles: vi.fn().mockResolvedValue([profile]), save: vi.fn().mockResolvedValue(profile), remove: vi.fn().mockResolvedValue(undefined), test: vi.fn().mockResolvedValue({ supported: true, models: ['image-test'] }), batches: vi.fn().mockResolvedValue([batch]), batch: vi.fn().mockResolvedValue(batch), cancel: vi.fn().mockResolvedValue({ ...batch, status: 'cancelled' }) } satisfies ImageApi
}
afterEach(cleanup)
it('编辑时密钥为空，留空保存不回传旧密钥', async () => {
  const service = api()
  render(<ImageSettings api={service} t={t} />)
  fireEvent.click(await screen.findByRole('button', { name: '编辑' }))
  const dialog = screen.getByRole('dialog')
  expect(within(dialog).getByLabelText<HTMLInputElement>('API Key').value).toBe('')
  fireEvent.click(within(dialog).getByRole('button', { name: '保存' }))
  await waitFor(() => { expect(service.save).toHaveBeenCalledOnce() })
  expect(service.save.mock.calls[0]?.[0]).not.toHaveProperty('apiKey')
})
it('保存失败保留表单并显示原因', async () => {
  const service = api(); service.save.mockRejectedValue(new Error('无法访问系统凭据管理器'))
  render(<ImageSettings api={service} t={t} />)
  fireEvent.click(await screen.findByRole('button', { name: '编辑' }))
  fireEvent.click(screen.getByRole('button', { name: '保存' }))
  expect((await screen.findByRole('alert')).textContent).toContain('无法访问系统凭据管理器')
  expect(screen.getByRole('dialog')).toBeTruthy()
})
it('按会话筛选结果，可取消正在运行的批次', async () => {
  const service = api(); service.batches.mockResolvedValue([batch, { ...batch, id: 'foreign', owner: 's2' }])
  render(<ImageResults api={service} t={t} owner="s1" />)
  await screen.findByText('批次 #1')
  expect(screen.queryByText('foreign')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: '取消批次' }))
  await waitFor(() => { expect(service.cancel).toHaveBeenCalledWith('b1') })
})
it('对话卡片从已记录结果提取批次 ID，错误不会被当成批次', () => {
  expect(imageBatchId('{"batchId":"b1","jobId":"j1"}')).toBe('b1')
  expect(imageBatchId('提交失败')).toBeUndefined()
  expect(imageBatchId('{"batchId":1}')).toBeUndefined()
})

it('打开指定批次时显示缩略图和原图链接，不查询其他批次', async () => {
  const service = api()
  const result = asset(1, { title: '生成的主图', filePath: 'assets/result.png', thumbPath: 'thumbs/result.webp' })
  service.batch.mockResolvedValue({ ...batch, status: 'completed', succeeded: 1, total: 1, assets: [result] })
  render(<ImageResults api={service} t={t} batchId="b1" />)
  const picture = await screen.findByRole('img', { name: '生成的主图' })
  expect(picture.getAttribute('src')).toBe(fileUrl('thumbs/result.webp'))
  expect(picture.closest('a')?.getAttribute('href')).toBe(fileUrl('assets/result.png'))
  expect(service.batch).toHaveBeenCalledWith('b1')
  expect(service.batches).not.toHaveBeenCalled()
  expect(screen.queryByRole('button', { name: '取消批次' })).toBeNull()
})
