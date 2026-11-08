/** 生图界面的 Remote 封装。 */
import type { RemoteResult } from '@deepseek-ai/dsh-typert-protocol'
import type { ImageProfile, ImageProfileInput, ImageBatch } from '@prism/dsh-host-catalog/types'
import { unwrap } from './api.ts'

/** 生图界面需要的生成接口。 */
export interface ImageRemote {
  listImageProfiles(): Promise<RemoteResult<readonly ImageProfile[]>>
  saveImageProfile(input: ImageProfileInput): Promise<RemoteResult<ImageProfile>>
  deleteImageProfile(request: { readonly id: string }): Promise<RemoteResult<void>>
  testImageProfile(
    request: { readonly id: string },
  ): Promise<RemoteResult<{ readonly supported: boolean; readonly models: readonly string[] }>>
  imageBatchStatus(request: { readonly id: string }): Promise<RemoteResult<ImageBatch>>
  listImageBatches(): Promise<RemoteResult<readonly ImageBatch[]>>
  cancelImageBatch(request: { readonly id: string }): Promise<RemoteResult<ImageBatch>>
}
/** 去掉传输包装后的界面接口。 */
export interface ImageApi {
  profiles(): Promise<readonly ImageProfile[]>
  save(input: ImageProfileInput): Promise<ImageProfile>
  remove(id: string): Promise<void>
  test(id: string): Promise<{ readonly supported: boolean; readonly models: readonly string[] }>
  batch(id: string): Promise<ImageBatch>
  batches(): Promise<readonly ImageBatch[]>
  cancel(id: string): Promise<ImageBatch>
}
/**
 * 将 Remote 业务错误转成界面可显示的错误。
 * @param remote - 生成的 Remote。
 * @param unreachable - 连接错误文案。
 * @returns 生图接口。
 */
export function createImageApi(remote: ImageRemote, unreachable: string): ImageApi {
  return {
    profiles: () => unwrap(remote.listImageProfiles(), unreachable),
    save: input => unwrap(remote.saveImageProfile(input), unreachable),
    remove: id => unwrap(remote.deleteImageProfile({ id }), unreachable),
    test: id => unwrap(remote.testImageProfile({ id }), unreachable),
    batch: id => unwrap(remote.imageBatchStatus({ id }), unreachable),
    batches: () => unwrap(remote.listImageBatches(), unreachable),
    cancel: id => unwrap(remote.cancelImageBatch({ id }), unreachable),
  }
}
