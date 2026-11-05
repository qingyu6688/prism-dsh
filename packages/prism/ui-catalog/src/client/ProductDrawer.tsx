/** 商品详情抽屉：在商品库页面右侧，不跳页。 */
import { useEffect, useState } from 'react'
import { Button, IconArchiveOutlineRegular, IconCloseOutlineRegular, IconEditOutlineRegular, IconTrashOutlineRegular, IconUnarchiveOutlineRegular, Modal, Tag } from '@deepseek-ai/dsh-client-ui-primitives'
import type { Product, ProductDetail } from '@prism/dsh-host-catalog/types'
import { messageOf, type CatalogApi } from './api.ts'
import { formatMoney } from './format.ts'
import type { Translate } from './ProductForm.tsx'
import css from './Catalog.module.css'

export interface ProductDrawerProps {
  readonly productId: string
  readonly api: CatalogApi
  readonly t: Translate
  /** 每次商品被改过就变一次，抽屉据此重新加载。 */
  readonly revision: number
  readonly onEdit: (product: Product) => void
  /** 归档、恢复、删除之后调用，页面刷新列表。 */
  readonly onChanged: () => void
  readonly onClose: () => void
}

type Load =
  | { readonly status: 'loading' }
  | { readonly status: 'ready'; readonly detail: ProductDetail }
  | { readonly status: 'error'; readonly message: string }

/**
 * 详情：基本信息、概况（规格、图片、文案数量）、卖点、参数、规格表、说明，以及编辑、归档、删除。
 * @param props - 商品 id、接口和回调。
 * @returns 抽屉内容。
 */
export function ProductDrawer({ productId, api, t, revision, onEdit, onChanged, onClose }: ProductDrawerProps) {
  const [load, setLoad] = useState<Load>({ status: 'loading' })
  const [reloads, setReloads] = useState(0)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

  useEffect(() => {
    let current = true
    setLoad({ status: 'loading' })
    setActionError(null)
    api.productDetail(productId).then(
      (detail) => { if (current) setLoad({ status: 'ready', detail }) },
      (error: unknown) => { if (current) setLoad({ status: 'error', message: messageOf(error) }) },
    )
    return () => { current = false }
  }, [api, productId, revision, reloads])

  const act = async (run: () => Promise<void>, closeAfter: boolean): Promise<void> => {
    setBusy(true)
    setActionError(null)
    try {
      await run()
      onChanged()
      if (closeAfter) onClose()
      else setReloads(n => n + 1)
    } catch (error) {
      setActionError(t('detail.actionError', { message: messageOf(error) }))
    } finally {
      setBusy(false)
      setConfirmDelete(false)
    }
  }

  const header = (title: string) => (
    <div className={css.drawerHead}>
      <h2 className={css.drawerTitle}>{title}</h2>
      <Button variant="ghost" size="sm" aria-label={t('close')} icon={<IconCloseOutlineRegular size={16} />} onClick={onClose} />
    </div>
  )

  if (load.status === 'loading') {
    return <aside className={css.drawer} aria-busy="true">{header('')}<p className={css.muted}>{t('detail.loading')}</p></aside>
  }
  if (load.status === 'error') {
    return (
      <aside className={css.drawer}>
        {header('')}
        <p className={css.errorText} role="alert">{load.message}</p>
        <Button variant="outline" size="sm" onClick={() => { setReloads(n => n + 1) }}>{t('retry')}</Button>
      </aside>
    )
  }

  const { product, skus, summary } = load.detail
  const copies = summary.copies.reduce((sum, row) => sum + row.total, 0)
  const value = (text: string | null): string => text ?? t('detail.none')
  return (
    <aside className={css.drawer} aria-label={product.name}>
      {header(product.name)}
      <div className={css.drawerActions}>
        <Button variant="outline" size="sm" icon={<IconEditOutlineRegular size={14} />} disabled={busy}
          onClick={() => { onEdit(product) }}>{t('edit')}</Button>
        {product.archived
          ? <Button variant="outline" size="sm" icon={<IconUnarchiveOutlineRegular size={14} />} disabled={busy}
            onClick={() => { void act(() => api.restoreProduct(product.id), false) }}>{t('detail.restore')}</Button>
          : <Button variant="outline" size="sm" icon={<IconArchiveOutlineRegular size={14} />} disabled={busy}
            onClick={() => { void act(() => api.archiveProduct(product.id), false) }}>{t('detail.archive')}</Button>}
        <Button variant="ghost" size="sm" icon={<IconTrashOutlineRegular size={14} />} disabled={busy}
          className={css.dangerButton} onClick={() => { setConfirmDelete(true) }}>{t('delete')}</Button>
      </div>
      {actionError === null ? null : <p className={css.errorText} role="alert">{actionError}</p>}
      {product.archived ? <p className={css.notice}>{t('detail.archivedNote')}</p> : null}

      <dl className={css.facts}>
        <dt>{t('detail.code')}</dt><dd>{value(product.code)}</dd>
        <dt>{t('detail.category')}</dt><dd>{value(product.category)}</dd>
        <dt>{t('detail.brand')}</dt><dd>{value(product.brand)}</dd>
        <dt>{t('detail.cost')}</dt><dd>{formatMoney(product.costCents, product.currency)}</dd>
        <dt>{t('detail.markets')}</dt>
        <dd>{product.targetMarkets.length === 0 ? t('detail.none') : product.targetMarkets.join(', ')}</dd>
      </dl>

      <section className={css.drawerSection}>
        <h3 className={css.sectionHead}>{t('detail.summary')}</h3>
        <div className={css.chips}>
          <Tag tone="neutral">{t('detail.skus', { count: summary.skuCount })}</Tag>
          <Tag tone="neutral">{t('detail.images', { count: summary.imageAssets })}</Tag>
          <Tag tone="neutral">{t('detail.texts', { count: summary.textAssets })}</Tag>
          <Tag tone="neutral">{t('detail.copies', { count: copies })}</Tag>
        </div>
      </section>

      <section className={css.drawerSection}>
        <h3 className={css.sectionHead}>{t('detail.sellingPoints')}</h3>
        {product.sellingPoints.length === 0
          ? <p className={css.muted}>{t('detail.none')}</p>
          : <ul className={css.points}>{product.sellingPoints.map(point => <li key={point}>{point}</li>)}</ul>}
      </section>

      <section className={css.drawerSection}>
        <h3 className={css.sectionHead}>{t('detail.attributes')}</h3>
        {product.attributes.length === 0
          ? <p className={css.muted}>{t('detail.none')}</p>
          : <dl className={css.facts}>{product.attributes.map(a => [
            <dt key={`${a.name}-k`}>{a.name}</dt>, <dd key={`${a.name}-v`}>{a.value}</dd>,
          ])}</dl>}
      </section>

      {skus.length === 0 ? null : (
        <section className={css.drawerSection}>
          <h3 className={css.sectionHead}>{t('detail.skuTable')}</h3>
          <table className={css.skuTable}>
            <thead><tr>
              <th>{t('detail.skuSpec')}</th><th>{t('detail.skuCode')}</th>
              <th>{t('detail.skuPrice')}</th><th>{t('detail.skuStock')}</th>
            </tr></thead>
            <tbody>{skus.map(sku => (
              <tr key={sku.id}>
                <td>{sku.spec.map(s => s.value).join(' / ')}</td>
                <td>{sku.code ?? '—'}</td>
                <td>{formatMoney(sku.priceCents, product.currency)}</td>
                <td>{sku.stock ?? '—'}</td>
              </tr>
            ))}</tbody>
          </table>
        </section>
      )}

      <section className={css.drawerSection}>
        <h3 className={css.sectionHead}>{t('detail.description')}</h3>
        <p className={css.description}>{value(product.description)}</p>
      </section>

      <Modal
        open={confirmDelete}
        tone="warning"
        closeLabel={t('close')}
        onClose={() => { setConfirmDelete(false) }}
        title={t('detail.deleteTitle', { name: product.name })}
        description={t('detail.deleteBody')}
        footer={<>
          <Button variant="outline" onClick={() => { setConfirmDelete(false) }}>{t('cancel')}</Button>
          <Button variant="primary" disabled={busy} onClick={() => { void act(() => api.deleteProduct(product.id), true) }}>{t('delete')}</Button>
        </>}
      />
    </aside>
  )
}
