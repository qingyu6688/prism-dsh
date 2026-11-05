/** 收藏按钮：空心星 / 实心星。dsh 的图标库里没有星星，按它的线宽画一个。 */
import clsx from 'clsx'
import type { Translate } from './ProductForm.tsx'
import css from './Catalog.module.css'

export interface StarButtonProps {
  readonly starred: boolean
  readonly t: Translate
  readonly onToggle: () => void
  readonly disabled?: boolean
  /** 按钮上带文字（详情面板里），否则只有图标（网格卡片上）。 */
  readonly withLabel?: boolean
  readonly className?: string | undefined
}

/**
 * 收藏按钮。
 * @param props - 是否已收藏、文案和回调。
 * @returns 按钮。
 */
export function StarButton({ starred, t, onToggle, disabled = false, withLabel = false, className }: StarButtonProps) {
  const label = t(starred ? 'assets.unstar' : 'assets.star')
  return (
    <button type="button" className={clsx(css.starButton, starred && css.starred, className)} aria-pressed={starred}
      aria-label={withLabel ? undefined : label} title={label} disabled={disabled}
      onClick={(event) => { event.stopPropagation(); onToggle() }}>
      <svg width={16} height={16} viewBox="0 0 24 24" fill={starred ? 'currentColor' : 'none'} stroke="currentColor"
        strokeWidth={1.5} strokeLinejoin="round" aria-hidden="true">
        <path d="m12 3.5 2.6 5.3 5.9.9-4.3 4.1 1 5.8-5.2-2.7-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z" />
      </svg>
      {withLabel ? <span>{label}</span> : null}
    </button>
  )
}
