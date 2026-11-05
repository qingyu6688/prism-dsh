/** 下拉选择：一个按钮加 dsh 的菜单。原界面规范要求不用系统原生下拉框。 */
import { useState } from 'react'
import { IconChevronDownOutlineRegular, Menu } from '@deepseek-ai/dsh-client-ui-primitives'
import css from './Catalog.module.css'

/** 一个选项。 */
export interface PickerOption {
  readonly id: string
  readonly label: string
}

export interface PickerProps {
  /** 按钮的 id，给 `<label htmlFor>` 关联用。 */
  readonly id: string
  readonly value: string | undefined
  readonly options: readonly PickerOption[]
  /** 没选时按钮上显示的字。 */
  readonly placeholder: string
  readonly onChange: (id: string) => void
  /** 没有关联 `<label>` 时的无障碍名称。 */
  readonly ariaLabel?: string | undefined
  readonly disabled?: boolean
}

/**
 * 下拉选择。
 * @param props - 当前值、选项和回调。
 * @returns 按钮和弹出的菜单。
 */
export function Picker({ id, value, options, placeholder, onChange, ariaLabel, disabled = false }: PickerProps) {
  const [open, setOpen] = useState(false)
  const current = options.find(option => option.id === value)
  return (
    <Menu
      open={open}
      onClose={() => { setOpen(false) }}
      portal
      minWidthFromAnchor
      className={css.pickerRoot}
      selectedId={current?.id}
      items={options.map(option => ({ id: option.id, label: option.label }))}
      onSelect={(next: string) => { onChange(next); setOpen(false) }}
      anchor={
        <button type="button" id={id} className={css.picker} aria-haspopup="listbox" aria-expanded={open}
          aria-label={ariaLabel} disabled={disabled} onClick={() => { setOpen(state => !state) }}>
          <span className={current === undefined ? css.pickerPlaceholder : css.pickerValue}>{current?.label ?? placeholder}</span>
          <IconChevronDownOutlineRegular size={14} />
        </button>
      }
    />
  )
}
