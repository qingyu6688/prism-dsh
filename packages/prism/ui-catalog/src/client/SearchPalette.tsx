/**
 * 全局搜索框（Ctrl+Shift+K，或商品库、素材库页头的搜索按钮）：搜商品和素材，上下键选、回车打开。
 * 商品打开商品库的详情抽屉；素材打开素材库并按同一个关键词筛选。会话搜索用 dsh 自带的（Ctrl+K）。
 */
import clsx from 'clsx'
import { useEffect, useState, useSyncExternalStore, type KeyboardEvent } from 'react'
import { IconSearchOutlineRegular, Input, Modal } from '@deepseek-ai/dsh-client-ui-primitives'
import type { SearchResults } from '@prism/dsh-host-catalog/types'
import { fileUrl, messageOf, type CatalogApi } from './api.ts'
import type { CatalogNavigator } from './navigator.ts'
import type { Translate } from './ProductForm.tsx'
import css from './Catalog.module.css'

const SEARCH_DELAY_MS = 200

export interface SearchPaletteProps {
  readonly api: CatalogApi
  readonly t: Translate
  readonly navigator: CatalogNavigator
}

/** 结果列表里的一项。 */
interface Row {
  readonly key: string
  readonly kind: 'product' | 'asset'
  readonly title: string
  readonly detail: string
  readonly thumb: string | null
  readonly open: () => void
}

type Load =
  | { readonly status: 'idle' }
  | { readonly status: 'loading' }
  | { readonly status: 'ready'; readonly results: SearchResults }
  | { readonly status: 'error'; readonly message: string }

/**
 * 搜索框。只在打开时渲染内容。
 * @param props - 接口、文案和跳转状态。
 * @returns 对话框。
 */
export function SearchPalette({ api, t, navigator }: SearchPaletteProps) {
  const { searchOpen } = useSyncExternalStore(navigator.subscribe, navigator.snapshot)
  const [keyword, setKeyword] = useState('')
  const [load, setLoad] = useState<Load>({ status: 'idle' })
  const [active, setActive] = useState(0)

  useEffect(() => {
    if (searchOpen) {
      setKeyword('')
      setLoad({ status: 'idle' })
    }
  }, [searchOpen])

  useEffect(() => {
    const trimmed = keyword.trim()
    if (!searchOpen || trimmed === '') {
      setLoad({ status: 'idle' })
      return
    }
    let current = true
    const timer = setTimeout(() => {
      setLoad({ status: 'loading' })
      api.search(trimmed).then(
        (results) => { if (current) { setLoad({ status: 'ready', results }); setActive(0) } },
        (error: unknown) => { if (current) setLoad({ status: 'error', message: messageOf(error) }) },
      )
    }, SEARCH_DELAY_MS)
    return () => { current = false; clearTimeout(timer) }
  }, [api, keyword, searchOpen])

  const rows: Row[] = load.status !== 'ready' ? [] : [
    ...load.results.products.map(product => ({
      key: `p-${product.id}`, kind: 'product' as const, title: product.name,
      detail: [product.code, product.category].filter(value => value !== null && value !== '').join(' · '),
      thumb: null, open: () => { navigator.showProduct(product.id) },
    })),
    ...load.results.assets.map(asset => ({
      key: `a-${asset.id}`, kind: 'asset' as const, title: asset.title ?? t('asset.untitled'),
      detail: asset.tags.join('、'), thumb: asset.thumbPath,
      open: () => { navigator.showAssets(keyword.trim()) },
    })),
  ]

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>): void => {
    if (rows.length === 0) return
    if (event.key === 'ArrowDown') { event.preventDefault(); setActive(index => (index + 1) % rows.length) }
    if (event.key === 'ArrowUp') { event.preventDefault(); setActive(index => (index - 1 + rows.length) % rows.length) }
    if (event.key === 'Enter') { event.preventDefault(); rows[active]?.open() }
  }

  const group = (kind: Row['kind'], title: string) => {
    const items = rows.filter(row => row.kind === kind)
    if (items.length === 0) return null
    return (
      <section className={css.searchGroup} aria-label={title}>
        <h3 className={css.sectionHead}>{title}</h3>
        <ul className={css.searchList}>{items.map((row) => {
          const index = rows.indexOf(row)
          return (
            <li key={row.key}>
              <button type="button" className={clsx(css.searchRow, index === active && css.searchRowActive)}
                aria-current={index === active} onMouseEnter={() => { setActive(index) }} onClick={row.open}>
                {row.thumb === null ? null : <img className={css.searchThumb} src={fileUrl(row.thumb)} alt="" />}
                <span className={css.productName}>{row.title}</span>
                <span className={css.productCode}>{row.detail}</span>
              </button>
            </li>
          )
        })}</ul>
      </section>
    )
  }

  return (
    <Modal open={searchOpen} onClose={() => { navigator.closeSearch() }} closeLabel={t('close')} title={t('search.title')}
      className={clsx(css.formModal)} shortcutModal="prism-search">
      <div className={css.form}>
        <Input data-modal-autofocus icon={<IconSearchOutlineRegular size={16} />} value={keyword} placeholder={t('search.placeholder')}
          aria-label={t('search.placeholder')} onKeyDown={onKeyDown} onChange={(event) => { setKeyword(event.target.value) }} />
        {load.status === 'idle' ? <p className={css.muted}>{t('search.hint')}</p> : null}
        {load.status === 'loading' ? <p className={css.muted}>{t('search.loading')}</p> : null}
        {load.status === 'error' ? <p className={css.errorText} role="alert">{t('search.error', { message: load.message })}</p> : null}
        {load.status === 'ready' && rows.length === 0 ? <p className={css.muted}>{t('search.empty')}</p> : null}
        {group('product', t('search.products'))}
        {group('asset', t('search.assets'))}
      </div>
    </Modal>
  )
}
