/**
 * 金额、日期的显示，以及商品表单的文字和接口字段之间的转换。
 * @module @prism/dsh-client-ui-catalog/client/format
 */

import type { Attribute, Product, ProductInput } from '@prism/dsh-host-catalog/types'

const CURRENCY_SYMBOLS: Readonly<Record<string, string>> = {
  CNY: '¥', USD: '$', EUR: '€', GBP: '£', JPY: '¥', HKD: 'HK$',
}

/**
 * 金额（分）显示成带币种符号的文字，如 `¥25.80`。
 * @param cents - 金额，单位分；`null` 显示为短横线。
 * @param currency - 币种代码。
 * @returns 显示文字。
 */
export function formatMoney(cents: number | null, currency: string): string {
  if (cents === null) return '—'
  const symbol = CURRENCY_SYMBOLS[currency]
  const amount = (cents / 100).toFixed(2)
  return symbol === undefined ? `${currency} ${amount}` : `${symbol}${amount}`
}

/**
 * 时间戳显示成本地日期，如 `2026-11-05`。
 * @param ms - Unix 毫秒。
 * @returns 日期文字。
 */
export function formatDate(ms: number): string {
  const date = new Date(ms)
  const pad = (n: number): string => String(n).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`
}

/** 商品表单里各输入框的文字。 */
export interface ProductDraft {
  name: string
  code: string
  category: string
  brand: string
  /** 成本，单位元，如 `25.8`。 */
  cost: string
  currency: string
  /** 目标市场，逗号或空格分隔，如 `US, UK`。 */
  markets: string
  /** 卖点，一行一条。 */
  sellingPoints: string
  /** 参数，一行一项，`名称：值`。 */
  attributes: string
  description: string
}

/** 新建商品时的空表单。 */
export const EMPTY_DRAFT: ProductDraft = {
  name: '', code: '', category: '', brand: '', cost: '', currency: 'CNY',
  markets: '', sellingPoints: '', attributes: '', description: '',
}

/**
 * 已有商品转成表单文字。
 * @param product - 商品。
 * @returns 表单。
 */
export function draftOf(product: Product): ProductDraft {
  return {
    name: product.name,
    code: product.code ?? '',
    category: product.category ?? '',
    brand: product.brand ?? '',
    cost: product.costCents === null ? '' : String(product.costCents / 100),
    currency: product.currency,
    markets: product.targetMarkets.join(', '),
    sellingPoints: product.sellingPoints.join('\n'),
    attributes: product.attributes.map(a => `${a.name}：${a.value}`).join('\n'),
    description: product.description ?? '',
  }
}

/** 表单校验出的问题，键是字段名，值是文案键。 */
export type DraftProblems = Partial<Record<keyof ProductDraft, 'required' | 'cost' | 'attribute'>>

function lines(text: string): string[] {
  return text.split(/\r?\n/).map(line => line.trim()).filter(line => line !== '')
}

function optional(text: string): string | null {
  const trimmed = text.trim()
  return trimmed === '' ? null : trimmed
}

/**
 * 参数文字解析成名称和值。一行一项，名称和值用中文或英文冒号、等号分开。
 * @param text - 输入框文字。
 * @returns 参数；有一行分不开时返回 `undefined`。
 */
export function parseAttributes(text: string): Attribute[] | undefined {
  const result: Attribute[] = []
  for (const line of lines(text)) {
    const [, name, value] = /^(.+?)\s*[:：=]\s*(.+)$/.exec(line) ?? []
    if (name === undefined || value === undefined) return undefined
    result.push({ name: name.trim(), value: value.trim() })
  }
  return result
}

/**
 * 成本文字（元）转成分。
 * @param text - 如 `25.8`、`¥25.80`；空表示不填。
 * @returns 分；空时为 `null`；格式不对时为 `undefined`。
 */
export function parseCost(text: string): number | null | undefined {
  const cleaned = text.trim().replace(/^[¥$€£]/, '').replaceAll(',', '')
  if (cleaned === '') return null
  if (!/^\d+(\.\d{1,2})?$/.test(cleaned)) return undefined
  return Math.round(Number(cleaned) * 100)
}

/**
 * 表单转成接口的输入，同时校验。货号唯一、目标市场代码等由服务端校验。
 * @param draft - 表单文字。
 * @returns 输入或问题。
 */
export function inputOf(draft: ProductDraft): { input: ProductInput } | { problems: DraftProblems } {
  const problems: DraftProblems = {}
  if (draft.name.trim() === '') problems.name = 'required'
  const costCents = parseCost(draft.cost)
  if (costCents === undefined) problems.cost = 'cost'
  const attributes = parseAttributes(draft.attributes)
  if (attributes === undefined) problems.attributes = 'attribute'
  if (Object.keys(problems).length > 0 || costCents === undefined || attributes === undefined) return { problems }
  return {
    input: {
      name: draft.name.trim(),
      code: optional(draft.code),
      category: optional(draft.category),
      brand: optional(draft.brand),
      description: optional(draft.description),
      sellingPoints: lines(draft.sellingPoints),
      attributes,
      costCents,
      currency: draft.currency.trim() === '' ? 'CNY' : draft.currency.trim().toUpperCase(),
      targetMarkets: draft.markets.split(/[\s,，、]+/).map(m => m.trim().toUpperCase()).filter(m => m !== ''),
    },
  }
}
