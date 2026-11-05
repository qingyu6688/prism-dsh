/** 素材详情面板：大图、尺寸来源、标签、关联商品、收藏、删除。 */
import { useEffect, useState } from 'react'
import { Button, IconCloseOutlineRegular, IconLinkOutlineRegular, IconTrashOutlineRegular, Input, Modal } from '@deepseek-ai/dsh-client-ui-primitives'
import type { Asset, Product } from '@prism/dsh-host-catalog/types'
import { fileUrl, messageOf, type CatalogApi } from './api.ts'
import { formatDate } from './format.ts'
import type { CatalogKey } from './locales.ts'
import type { Translate } from './ProductForm.tsx'
import { StarButton } from './StarButton.tsx'
import css from './Catalog.module.css'

/** 关联商品时搜索结果最多显示几条。 */
const PRODUCT_RESULTS = 8
const SEARCH_DELAY_MS = 250
const SOURCES = ['upload', 'generated', 'captured', 'imported']

export interface AssetPanelProps {
  readonly asset: Asset
  readonly api: CatalogApi
  readonly t: Translate
  /** 素材被改了（标签、收藏、关联），参数是改后的素材。 */
  readonly onChanged: (asset: Asset) => void
  readonly onDeleted: (id: string) => void
  readonly onClose: () => void
}

/**
 * 标签文字拆成标签：中英文逗号、顿号分开，去空白和重复。
 * @param text - 输入框文字。
 * @returns 标签。
 */
export function parseTags(text: string): string[] {
  return [...new Set(text.split(/[,，、]/).map(tag => tag.trim()).filter(tag => tag !== ''))]
}

/**
 * 素材详情面板。
 * @param props - 素材、接口和回调。
 * @returns 面板。
 */
export function AssetPanel({ asset, api, t, onChanged, onDeleted, onClose }: AssetPanelProps) {
  const [tags, setTags] = useState(asset.tags.join(', '))
  const [product, setProduct] = useState<Product | null>(null)
  const [linking, setLinking] = useState(false)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => { setTags(asset.tags.join(', ')) }, [asset.tags])

  // 关联的商品只有 id，取一下名称
  useEffect(() => {
    let current = true
    setProduct(null)
    if (asset.productId !== null) {
      api.productDetail(asset.productId).then(
        (detail) => { if (current) setProduct(detail.product) },
        // 商品可能已经删了，面板上照样显示"没有关联商品"，不算错误
        () => { if (current) setProduct(null) },
      )
    }
    return () => { current = false }
  }, [api, asset.productId])

  const act = async (run: () => Promise<Asset | void>, next: (asset: Asset) => Asset): Promise<void> => {
    setBusy(true)
    setError(null)
    try {
      const result = await run()
      onChanged(result ?? next(asset))
    } catch (failure) {
      setError(t('detail.actionError', { message: messageOf(failure) }))
    } finally {
      setBusy(false)
    }
  }

  const remove = async (): Promise<void> => {
    setBusy(true)
    try {
      await api.deleteAsset(asset.id)
      onDeleted(asset.id)
    } catch (failure) {
      setError(t('detail.actionError', { message: messageOf(failure) }))
      setBusy(false)
    } finally {
      setConfirmDelete(false)
    }
  }

  const title = asset.title ?? t('asset.untitled')
  const tagsChanged = parseTags(tags).join('\n') !== asset.tags.join('\n')
  return (
    <aside className={css.drawer} aria-label={title}>
      <div className={css.drawerHead}>
        <h2 className={css.drawerTitle}>{title}</h2>
        <Button variant="ghost" size="sm" aria-label={t('close')} icon={<IconCloseOutlineRegular size={16} />} onClick={onClose} />
      </div>
      {asset.kind === 'image' && asset.filePath !== null
        ? <img className={css.assetPreview} src={fileUrl(asset.filePath)} alt={title} />
        : <p className={css.assetText}>{asset.textContent ?? ''}</p>}
      <div className={css.drawerActions}>
        <StarButton starred={asset.starred} t={t} disabled={busy} withLabel
          onToggle={() => { void act(() => api.starAsset(asset.id, !asset.starred), a => ({ ...a, starred: !a.starred })) }} />
        <Button variant="ghost" size="sm" icon={<IconTrashOutlineRegular size={14} />} disabled={busy}
          className={css.dangerButton} onClick={() => { setConfirmDelete(true) }}>{t('delete')}</Button>
      </div>
      {error === null ? null : <p className={css.errorText} role="alert">{error}</p>}

      <dl className={css.facts}>
        {asset.width !== null && asset.height !== null
          ? <><dt>{t('asset.sizeLabel')}</dt><dd>{t('asset.size', { width: asset.width, height: asset.height })}</dd></>
          : null}
        <dt>{t('asset.source')}</dt>
        <dd>{SOURCES.includes(asset.source) ? t(`asset.source.${asset.source}` as CatalogKey) : asset.source}</dd>
        <dt>{t('asset.created')}</dt><dd>{formatDate(asset.createdAt)}</dd>
      </dl>

      <section className={css.drawerSection}>
        <label className={css.sectionHead} htmlFor="asset-tags">{t('asset.tags')}</label>
        <div className={css.inlineControls}>
          <Input id="asset-tags" value={tags} placeholder={t('asset.tagsHint')} onChange={(event) => { setTags(event.target.value) }} />
          <Button variant="outline" size="sm" disabled={busy || !tagsChanged}
            onClick={() => { void act(() => api.setAssetTags(asset.id, parseTags(tags)), a => a) }}>{t('asset.saveTags')}</Button>
        </div>
      </section>

      <section className={css.drawerSection}>
        <h3 className={css.sectionHead}>{t('asset.product')}</h3>
        {asset.productId === null
          ? <p className={css.muted}>{t('asset.noProduct')}</p>
          : <div className={css.inlineControls}>
            <span className={css.linkedProduct}>{product === null ? asset.productId : `${product.name}${product.code === null ? '' : ` · ${product.code}`}`}</span>
            <Button variant="ghost" size="sm" disabled={busy}
              onClick={() => { void act(() => api.linkAsset(asset.id, null), a => ({ ...a, productId: null })) }}>{t('asset.unlink')}</Button>
          </div>}
        {linking
          ? <ProductSearch api={api} t={t} onPick={(picked) => {
            setLinking(false)
            void act(() => api.linkAsset(asset.id, picked.id), a => ({ ...a, productId: picked.id }))
          }} />
          : <Button variant="outline" size="sm" icon={<IconLinkOutlineRegular size={14} />} disabled={busy} className={css.alignStart}
            onClick={() => { setLinking(true) }}>{t('asset.link')}</Button>}
      </section>

      <Modal open={confirmDelete} tone="warning" closeLabel={t('close')} onClose={() => { setConfirmDelete(false) }}
        title={t('asset.deleteTitle')} description={t('asset.deleteBody')}
        footer={<>
          <Button variant="outline" onClick={() => { setConfirmDelete(false) }}>{t('cancel')}</Button>
          <Button variant="primary" disabled={busy} onClick={() => { void remove() }}>{t('delete')}</Button>
        </>} />
    </aside>
  )
}

/** 关联商品时的搜索框和结果。 */
function ProductSearch({ api, t, onPick }: {
  readonly api: CatalogApi
  readonly t: Translate
  readonly onPick: (product: Product) => void
}) {
  const [keyword, setKeyword] = useState('')
  const [results, setResults] = useState<readonly Product[] | null>(null)

  useEffect(() => {
    let current = true
    const timer = setTimeout(() => {
      api.listProducts({ keyword: keyword.trim() === '' ? null : keyword.trim(), limit: PRODUCT_RESULTS }).then(
        (page) => { if (current) setResults(page.items) },
        () => { if (current) setResults([]) },
      )
    }, SEARCH_DELAY_MS)
    return () => { current = false; clearTimeout(timer) }
  }, [api, keyword])

  return (
    <div className={css.productSearch}>
      <Input autoFocus value={keyword} placeholder={t('asset.pickProduct')} aria-label={t('asset.pickProduct')}
        onChange={(event) => { setKeyword(event.target.value) }} />
      {results === null ? null : results.length === 0
        ? <p className={css.muted}>{t('asset.noProductMatch')}</p>
        : <ul className={css.productResults}>{results.map(product => (
          <li key={product.id}>
            <button type="button" className={css.productResult} onClick={() => { onPick(product) }}>
              <span>{product.name}</span>
              <span className={css.productCode}>{product.code ?? ''}</span>
            </button>
          </li>
        ))}</ul>}
    </div>
  )
}
