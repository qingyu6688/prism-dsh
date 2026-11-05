/** 素材库页面：缩略图网格、搜索、按标签和收藏筛选、导入图片、右侧详情面板。 */
import clsx from 'clsx'
import { useCallback, useEffect, useRef, useState } from 'react'
import { Button, Checkbox, IconPlusOutlineRegular, IconSearchOutlineRegular, Input } from '@deepseek-ai/dsh-client-ui-primitives'
import type { Asset, TagCount } from '@prism/dsh-host-catalog/types'
import { fileUrl, messageOf, type CatalogApi } from './api.ts'
import { AssetPanel } from './AssetPanel.tsx'
import { Picker } from './Picker.tsx'
import type { Translate } from './ProductForm.tsx'
import { StarButton } from './StarButton.tsx'
import css from './Catalog.module.css'

/** 每次加载多少个素材。网格一屏放得下几十个，多了点"加载更多"。 */
export const ASSET_PAGE_SIZE = 60
const SEARCH_DELAY_MS = 250
const IMAGE_ACCEPT = '.png,.jpg,.jpeg,.webp'
/** 标签筛选里"全部标签"的 id，不会和标签重名。 */
const ALL_TAGS = '\u0000all'

export interface AssetsPageProps {
  readonly api: CatalogApi
  readonly t: Translate
}

type Load =
  | { readonly status: 'loading' }
  | { readonly status: 'ready'; readonly items: readonly Asset[]; readonly total: number }
  | { readonly status: 'error'; readonly message: string }

/** 一次导入的结果，显示在列表上方。 */
interface ImportReport {
  readonly imported: number
  readonly deduplicated: number
  readonly failed: readonly { readonly name: string; readonly message: string }[]
}

/**
 * 素材库页面。
 * @param props - 接口和文案。
 * @returns 页面。
 */
export function AssetsPage({ api, t }: AssetsPageProps) {
  const [search, setSearch] = useState('')
  const [keyword, setKeyword] = useState('')
  const [tag, setTag] = useState<string | null>(null)
  const [starredOnly, setStarredOnly] = useState(false)
  const [load, setLoad] = useState<Load>({ status: 'loading' })
  const [loadingMore, setLoadingMore] = useState(false)
  const [tags, setTags] = useState<readonly TagCount[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null)
  const [report, setReport] = useState<ImportReport | null>(null)
  const [reloads, setReloads] = useState(0)
  const fileInput = useRef<HTMLInputElement | null>(null)

  useEffect(() => {
    const timer = setTimeout(() => { setKeyword(search) }, SEARCH_DELAY_MS)
    return () => { clearTimeout(timer) }
  }, [search])

  const filter = useCallback((offset: number) => ({
    keyword: keyword.trim() === '' ? null : keyword.trim(),
    tag,
    starredOnly,
    limit: ASSET_PAGE_SIZE,
    offset,
  }), [keyword, tag, starredOnly])

  useEffect(() => {
    let current = true
    setLoad({ status: 'loading' })
    api.listAssets(filter(0)).then(
      (page) => { if (current) setLoad({ status: 'ready', items: page.items, total: page.total }) },
      (error: unknown) => { if (current) setLoad({ status: 'error', message: messageOf(error) }) },
    )
    return () => { current = false }
  }, [api, filter, reloads])

  useEffect(() => {
    let current = true
    api.listAssetTags().then(
      (list) => { if (current) setTags(list) },
      // 标签只用来筛选，取不到时筛选菜单只剩"全部标签"，列表照常显示
      () => { if (current) setTags([]) },
    )
    return () => { current = false }
  }, [api, reloads])

  const loadMore = async (): Promise<void> => {
    if (load.status !== 'ready') return
    setLoadingMore(true)
    try {
      const page = await api.listAssets(filter(load.items.length))
      setLoad({ status: 'ready', items: [...load.items, ...page.items], total: page.total })
    } catch (error) {
      setLoad({ status: 'error', message: messageOf(error) })
    } finally {
      setLoadingMore(false)
    }
  }

  const importFiles = async (files: readonly File[]): Promise<void> => {
    if (files.length === 0) return
    setReport(null)
    const failed: { name: string; message: string }[] = []
    const paths: string[] = []
    const names = new Map<string, string>()
    for (const [index, file] of files.entries()) {
      setProgress({ done: index, total: files.length })
      try {
        const uploaded = await api.upload(file)
        paths.push(uploaded.path)
        names.set(uploaded.path, file.name)
      } catch (error) {
        failed.push({ name: file.name, message: messageOf(error) })
      }
    }
    setProgress({ done: files.length, total: files.length })
    let imported = 0
    let deduplicated = 0
    if (paths.length > 0) {
      try {
        const result = await api.importAssetFiles({ paths, tags: [] })
        imported = result.imported.length
        deduplicated = result.imported.filter(item => item.deduplicated).length
        for (const item of result.failed) failed.push({ name: names.get(item.path) ?? item.path, message: item.message })
      } catch (error) {
        for (const path of paths) failed.push({ name: names.get(path) ?? path, message: messageOf(error) })
      }
    }
    setProgress(null)
    setReport({ imported, deduplicated, failed })
    setReloads(n => n + 1)
  }

  const replace = (changed: Asset): void => {
    if (load.status !== 'ready') return
    setLoad({ ...load, items: load.items.map(item => item.id === changed.id ? changed : item) })
    if (changed.tags.join('\n') !== load.items.find(item => item.id === changed.id)?.tags.join('\n')) {
      // 标签改了，筛选菜单里的计数也要跟着变
      api.listAssetTags().then(setTags, () => { setTags([]) })
    }
  }

  const selected = load.status === 'ready' ? load.items.find(item => item.id === selectedId) : undefined
  const filtered = keyword.trim() !== '' || tag !== null || starredOnly
  const tagOptions = [{ id: ALL_TAGS, label: t('assets.allTags') }, ...tags.map(row => ({ id: row.tag, label: `${row.tag}（${row.count}）` }))]

  const body = (() => {
    if (load.status === 'loading') return <p className={css.stateText}>{t('assets.loading')}</p>
    if (load.status === 'error') {
      return (
        <div className={css.stateBox} role="alert">
          <p>{t('assets.loadError', { message: load.message })}</p>
          <Button variant="outline" size="sm" onClick={() => { setReloads(n => n + 1) }}>{t('retry')}</Button>
        </div>
      )
    }
    if (load.items.length === 0) {
      if (filtered) return <p className={css.stateText}>{t('assets.noMatch')}</p>
      return (
        <div className={css.emptyBox}>
          <p className={css.emptyTitle}>{t('assets.emptyTitle')}</p>
          <p className={css.muted}>{t('assets.emptyBody')}</p>
          <Button variant="primary" size="sm" icon={<IconPlusOutlineRegular size={14} />}
            onClick={() => { fileInput.current?.click() }}>{t('assets.import')}</Button>
        </div>
      )
    }
    return (
      <div className={css.assetScroll}>
        <ul className={css.assetGrid}>{load.items.map(asset => (
          <li key={asset.id}>
            <div className={clsx(css.assetCard, asset.id === selectedId && css.selectedCard)}>
              <button type="button" className={css.assetOpen} aria-pressed={asset.id === selectedId}
                aria-label={asset.title ?? t('asset.untitled')} onClick={() => { setSelectedId(asset.id) }}>
                {asset.kind === 'image' && asset.thumbPath !== null
                  ? <img className={css.assetThumb} src={fileUrl(asset.thumbPath)} alt="" loading="lazy" />
                  : <span className={css.assetTextThumb}>{asset.textContent ?? t('asset.text')}</span>}
                <span className={css.assetCaption}>
                  <span className={css.assetTitle}>{asset.title ?? t('asset.untitled')}</span>
                  {asset.width !== null && asset.height !== null
                    ? <span className={css.productCode}>{t('asset.size', { width: asset.width, height: asset.height })}</span>
                    : null}
                </span>
              </button>
              <StarButton starred={asset.starred} t={t} className={css.cardStar}
                onToggle={() => {
                  api.starAsset(asset.id, !asset.starred).then(
                    () => { replace({ ...asset, starred: !asset.starred }) },
                    (error: unknown) => { setLoad({ status: 'error', message: messageOf(error) }) },
                  )
                }} />
            </div>
          </li>
        ))}</ul>
        {load.items.length < load.total
          ? <div className={css.loadMore}>
            <Button variant="outline" size="sm" disabled={loadingMore} onClick={() => { void loadMore() }}>{t('assets.loadMore')}</Button>
          </div>
          : null}
      </div>
    )
  })()

  return (
    <div className={css.page}>
      <div className={css.listPane}>
        <header className={css.pageHead}>
          <div className={css.titleLine}>
            <h1 className={css.pageTitle}>{t('assets.title')}</h1>
            {load.status === 'ready' ? <span className={css.muted}>{t('assets.count', { count: load.total })}</span> : null}
          </div>
          <input ref={fileInput} type="file" accept={IMAGE_ACCEPT} multiple hidden aria-label={t('assets.import')}
            onChange={(event) => { void importFiles([...(event.target.files ?? [])]); event.target.value = '' }} />
          <Button variant="primary" size="sm" icon={<IconPlusOutlineRegular size={14} />} disabled={progress !== null}
            onClick={() => { fileInput.current?.click() }}>
            {progress === null ? t('assets.import') : t('assets.importing', { done: progress.done, total: progress.total })}
          </Button>
        </header>
        {report === null ? null : <ImportNotice report={report} t={t} onClose={() => { setReport(null) }} />}
        <div className={css.toolbar}>
          <Input className={clsx(css.search)} icon={<IconSearchOutlineRegular size={16} />} value={search}
            placeholder={t('assets.search')} aria-label={t('assets.search')}
            onChange={(event) => { setSearch(event.target.value) }} />
          <div className={css.tagFilter}>
            <Picker id="prism-asset-tag" value={tag ?? ALL_TAGS} placeholder={t('assets.allTags')} options={tagOptions}
              ariaLabel={t('asset.tags')} onChange={(next) => { setTag(next === ALL_TAGS ? null : next) }} />
          </div>
          <Checkbox checked={starredOnly} onChange={setStarredOnly} label={t('assets.starredOnly')} />
        </div>
        <div className={css.listBody}
          onDragOver={(event) => { event.preventDefault() }}
          onDrop={(event) => { event.preventDefault(); void importFiles([...event.dataTransfer.files]) }}>
          {body}
        </div>
      </div>
      {selected === undefined ? null : (
        <AssetPanel key={selected.id} asset={selected} api={api} t={t}
          onChanged={replace}
          onDeleted={() => { setSelectedId(null); setReloads(n => n + 1) }}
          onClose={() => { setSelectedId(null) }} />
      )}
    </div>
  )
}

function ImportNotice({ report, t, onClose }: { readonly report: ImportReport; readonly t: Translate; readonly onClose: () => void }) {
  return (
    <div className={clsx(css.importNotice, report.failed.length > 0 && css.importNoticeWarn)} role="status">
      <div className={css.inlineControls}>
        <span className={css.noticeParts}>
          <span>{t('assets.imported', { count: report.imported })}</span>
          {report.deduplicated > 0 ? <span>{t('assets.importedDup', { count: report.deduplicated })}</span> : null}
          {report.failed.length > 0 ? <span>{t('assets.importFailed', { count: report.failed.length })}</span> : null}
        </span>
        <Button variant="ghost" size="sm" onClick={onClose}>{t('close')}</Button>
      </div>
      {report.failed.length === 0 ? null : (
        <ul className={css.problemList}>{report.failed.map(item => (
          <li key={item.name}><span className={css.problemLine}>{item.name}</span><span>{item.message}</span></li>
        ))}</ul>
      )}
    </div>
  )
}
