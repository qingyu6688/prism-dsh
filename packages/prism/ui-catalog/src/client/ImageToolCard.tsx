/** 会话中的生图卡片以已记录的批次 ID 查询持久化结果。 */
import type { ToolCallViewProps } from '@deepseek-ai/dsh-client-ui-tool/client'
import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import type { ImageApi } from './image-api.ts'
import { ImageResults } from './Images.tsx'

/**
 * 从工具结果读取批次引用，旧记录或失败结果保留原文。
 * @param text - 会话中记录的工具输出。
 * @returns 有效批次标识，其他输出返回 undefined。
 */
export function imageBatchId(text: string): string | undefined {
  let value: unknown
  try { value = JSON.parse(text) } catch (error) {
    // 工具错误是普通文本；没有可查询的批次。
    if (error instanceof SyntaxError) return undefined
    throw error
  }
  return typeof value === 'object' && value !== null && 'batchId' in value && typeof value.batchId === 'string'
    ? value.batchId : undefined
}

/**
 * 工具完成后从持久化批次恢复图片；错误结果保留原文。
 * @param props - 已记录的调用、结果与生图接口。
 * @returns 批次图片或执行状态。
 */
export function ImageToolCard(props: ToolCallViewProps & PropsLocale<'prism.catalog'> & { readonly api: ImageApi }) {
  if (props.phase !== 'result') return <p role="status">{props.t('image.submitting')}</p>
  const text = props.block.content.filter(block => block.type === 'text').map(block => block.text).join('\n')
  const id = props.block.isError ? undefined : imageBatchId(text)
  return id === undefined ? <p role={props.block.isError ? 'alert' : 'status'}>{text}</p>
    : <ImageResults api={props.api} t={props.t} batchId={id} />
}
