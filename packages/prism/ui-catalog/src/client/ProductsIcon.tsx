/** 左侧导航"商品库"的图标：一个包装盒。dsh 的图标库里没有合适的，按它的线宽（1.5）画一个。 */
import type { PropsRuntime } from '@deepseek-ai/dsh-client-ui-slots'
import type {} from '@deepseek-ai/dsh-client-ui-sidebar/client'

/**
 * 按侧栏给的尺寸画图标；无障碍名称由侧栏的导航文字提供。
 * @param props - 侧栏给的尺寸。
 * @returns 装饰性图标。
 */
export function ProductsIcon({ size }: PropsRuntime<'sidebar.panellist'>) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5}
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M3.5 7.5 12 3l8.5 4.5v9L12 21l-8.5-4.5z" />
      <path d="M3.5 7.5 12 12l8.5-4.5M12 12v9" />
      <path d="m7.75 5.25 8.5 4.5" />
    </svg>
  )
}

/**
 * 左侧导航"素材库"的图标：一张图片。
 * @param props - 侧栏给的尺寸。
 * @returns 装饰性图标。
 */
export function AssetsIcon({ size }: PropsRuntime<'sidebar.panellist'>) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5}
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="3.5" y="4.5" width="17" height="15" rx="2.5" />
      <circle cx="9" cy="10" r="1.75" />
      <path d="m20.5 16-4.5-4.5-8.5 8" />
    </svg>
  )
}

/**
 * 设置里"规则包"的图标：一份带勾的清单。
 * @param props - 设置页给的尺寸。
 * @returns 装饰性图标。
 */
export function RulesIcon({ size }: { readonly size: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5}
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M7 3.5h10A2.5 2.5 0 0 1 19.5 6v12a2.5 2.5 0 0 1-2.5 2.5H7A2.5 2.5 0 0 1 4.5 18V6A2.5 2.5 0 0 1 7 3.5z" />
      <path d="m8 9 1.5 1.5L12 8M8 15l1.5 1.5L12 14M14.5 9.5H16M14.5 15.5H16" />
    </svg>
  )
}

/**
 * 设置里"店铺"的图标：一个店面。
 * @param props - 设置页给的尺寸。
 * @returns 装饰性图标。
 */
export function ShopIcon({ size }: { readonly size: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5}
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M4 9.5V20h16V9.5" />
      <path d="M3 9.5 5 4h14l2 5.5c0 1.4-1.1 2.5-2.5 2.5S16 10.9 16 9.5c0 1.4-1.1 2.5-2.5 2.5h-3C9.1 12 8 10.9 8 9.5 8 10.9 6.9 12 5.5 12S3 10.9 3 9.5z" />
      <path d="M10 20v-5h4v5" />
    </svg>
  )
}
