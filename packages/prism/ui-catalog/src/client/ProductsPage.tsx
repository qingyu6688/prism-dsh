/** 商品库页面：左侧导航进入的全局页面，搜索、在用 / 已归档、虚拟滚动的列表、右侧详情抽屉。 */
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import clsx from 'clsx'
import { Button, IconPlusOutlineRegular, IconSearchOutlineRegular, Input, SegmentedControl } from '@deepseek-ai/dsh-client-ui-primitives'
import type { Product } from '@prism/dsh-host-catalog/types'
import type { CatalogApi } from './api.ts'
import { formatDate, formatMoney } from './format.ts'
import { ImportDialog } from './ImportDialog.tsx'
import { ProductDrawer } from './ProductDrawer.tsx'
import { ProductForm, type Translate } from './ProductForm.tsx'
import { ProductRows } from './product-rows.ts'
import css from './Catalog.module.css'

/** 行高固定，虚拟列表不用测量。 */
export const ROW_HEIGHT = 52
/** 搜索框停止输入多久后再查。 */
const SEARCH_DELAY_MS = 250

/** 页面需要的接口和文案。 */
export interface ProductsPageProps {
  readonly api: CatalogApi
  readonly t: Translate
}

type Status = 'active' | 'archived'

/**
 * 商品库页面。
 * @param props - 接口和文案。
 * @returns 页面。
 */
export function ProductsPage({ api, t }: ProductsPageProps) {
  const rows = useMemo(() => new ProductRows(filter => api.listProducts(filter)), [api])
  const snapshot = useSyncExternalStore(rows.subscribe, rows.snapshot)
  const [search, setSearch] = useState('')
  const [keyword, setKeyword] = useState('')
  const [status, setStatus] = useState<Status>('active')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [revision, setRevision] = useState(0)
  const [form, setForm] = useState<{ open: boolean; product?: Product | undefined }>({ open: false })
  const [importing, setImporting] = useState(false)

  useEffect(() => {
    const timer = setTimeout(() => { setKeyword(search) }, SEARCH_DELAY_MS)
    return () => { clearTimeout(timer) }
  }, [search])

  useEffect(() => {
    rows.setFilter({ keyword, archived: status === 'archived' })
  }, [rows, keyword, status])

  const scrollRef = useRef<HTMLDivElement | null>(null)
  const virtualizer = useVirtualizer({
    count: snapshot.status === 'ready' ? snapshot.total : 0,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
    // 测量到滚动区域之前先按这个大小算（也让测试环境里能渲染出行）
    initialRect: { width: 960, height: 640 },
  })
  const visible = virtualizer.getVirtualItems()
  const first = visible[0]?.index
  const last = visible.at(-1)?.index
  useEffect(() => {
    if (first !== undefined && last !== undefined) rows.ensure(first, last)
  }, [rows, first, last, snapshot.version])

  const changed = (): void => {
    rows.reload()
    setRevision(n => n + 1)
  }

  const filtered = keyword.trim() !== ''
  const body = (() => {
    if (snapshot.status === 'loading' && snapshot.total === 0) return <p className={css.stateText}>{t('products.loading')}</p>
    if (snapshot.status === 'error') {
      return (
        <div className={css.stateBox} role="alert">
          <p>{t('products.loadError', { message: snapshot.error ?? '' })}</p>
          <Button variant="outline" size="sm" onClick={() => { rows.reload() }}>{t('retry')}</Button>
        </div>
      )
    }
    if (snapshot.total === 0) {
      if (filtered) return <p className={css.stateText}>{t('products.noMatch')}</p>
      if (status === 'archived') return <p className={css.stateText}>{t('products.noArchived')}</p>
      return (
        <div className={css.emptyBox}>
          <p className={css.emptyTitle}>{t('products.emptyTitle')}</p>
          <p className={css.muted}>{t('products.emptyBody')}</p>
          <div className={css.emptyActions}>
            <Button variant="primary" size="sm" icon={<IconPlusOutlineRegular size={14} />}
              onClick={() => { setForm({ open: true }) }}>{t('products.new')}</Button>
            <Button variant="outline" size="sm" onClick={() => { setImporting(true) }}>{t('products.import')}</Button>
          </div>
        </div>
      )
    }
    return (
      <div className={css.table} role="table" aria-rowcount={snapshot.total}>
        <div className={clsx(css.row, css.headRow)} role="row">
          <span role="columnheader">{t('products.colProduct')}</span>
          <span role="columnheader">{t('products.colCategory')}</span>
          <span role="columnheader" className={css.wideOnly}>{t('products.colBrand')}</span>
          <span role="columnheader" className={css.numeric}>{t('products.colCost')}</span>
          <span role="columnheader" className={css.wideOnly}>{t('products.colMarkets')}</span>
          <span role="columnheader">{t('products.colUpdated')}</span>
        </div>
        <div className={css.rowsScroll} ref={scrollRef}>
          <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }} role="rowgroup">
            {visible.map((item) => {
              const product = rows.get(item.index)
              const style = { position: 'absolute' as const, top: 0, left: 0, right: 0, height: ROW_HEIGHT, transform: `translateY(${item.start}px)` }
              if (product === undefined) {
                return <div key={item.key} className={clsx(css.row, css.placeholderRow)} style={style} role="row" aria-rowindex={item.index + 2} />
              }
              return (
                <button type="button" key={item.key} style={style} role="row" aria-rowindex={item.index + 2}
                  className={clsx(css.row, css.dataRow, product.id === selectedId && css.selectedRow)}
                  aria-pressed={product.id === selectedId}
                  onClick={() => { setSelectedId(product.id) }}>
                  <span role="cell" className={css.productCell}>
                    <span className={css.productName}>{product.name}</span>
                    <span className={css.productCode}>{product.code ?? ''}</span>
                  </span>
                  <span role="cell" className={css.cellText}>{product.category ?? '—'}</span>
                  <span role="cell" className={clsx(css.cellText, css.wideOnly)}>{product.brand ?? '—'}</span>
                  <span role="cell" className={css.numeric}>{formatMoney(product.costCents, product.currency)}</span>
                  <span role="cell" className={clsx(css.cellText, css.wideOnly)}>{product.targetMarkets.join(', ') || '—'}</span>
                  <span role="cell" className={css.cellText}>{formatDate(product.updatedAt)}</span>
                </button>
              )
            })}
          </div>
        </div>
      </div>
    )
  })()

  return (
    <div className={css.page}>
      <div className={css.listPane}>
        <header className={css.pageHead}>
          <div className={css.titleLine}>
            <h1 className={css.pageTitle}>{t('products.title')}</h1>
            {snapshot.status === 'ready' ? <span className={css.muted}>{t('products.count', { count: snapshot.total })}</span> : null}
          </div>
          <div className={css.headActions}>
            <Button variant="outline" size="sm" onClick={() => { setImporting(true) }}>{t('products.import')}</Button>
            <Button variant="primary" size="sm" icon={<IconPlusOutlineRegular size={14} />}
              onClick={() => { setForm({ open: true }) }}>{t('products.new')}</Button>
          </div>
        </header>
        <div className={css.toolbar}>
          <Input className={clsx(css.search)} icon={<IconSearchOutlineRegular size={16} />} value={search}
            placeholder={t('products.search')} aria-label={t('products.search')}
            onChange={(event) => { setSearch(event.target.value) }} />
          <SegmentedControl id="prism-products-status" label={t('products.filter')} value={status}
            options={[{ value: 'active', label: t('products.active') }, { value: 'archived', label: t('products.archived') }]}
            onChange={(next) => { setStatus(next); setSelectedId(null) }} />
        </div>
        <div className={css.listBody}>{body}</div>
      </div>
      {selectedId === null ? null : (
        <ProductDrawer key={selectedId} productId={selectedId} api={api} t={t} revision={revision}
          onEdit={(product) => { setForm({ open: true, product }) }}
          onChanged={changed}
          onClose={() => { setSelectedId(null) }} />
      )}
      <ProductForm open={form.open} product={form.product} api={api} t={t}
        onClose={() => { setForm({ open: false }) }}
        onSaved={(product) => {
          setForm({ open: false })
          setSelectedId(product.id)
          changed()
        }} />
      <ImportDialog open={importing} api={api} t={t}
        onClose={(imported) => {
          setImporting(false)
          if (imported) changed()
        }} />
    </div>
  )
}
