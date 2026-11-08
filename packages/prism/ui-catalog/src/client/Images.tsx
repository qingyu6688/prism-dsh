/** 生图配置与持久化结果。所有请求由接口层提供，卸载后停止查询。 */
import { useEffect, useState } from 'react'
import { Button, Input, Modal } from '@deepseek-ai/dsh-client-ui-primitives'
import type { ImageProfileInput, ImageProfile, ImageBatch } from '@prism/dsh-host-catalog/types'
import type { ImageApi } from './image-api.ts'
import { fileUrl, messageOf } from './api.ts'
import { Field, type Translate } from './ProductForm.tsx'
import { Picker } from './Picker.tsx'
import css from './Images.module.css'

/** 生图页面共用参数。 */
export interface ImageProps {
  readonly api: ImageApi
  readonly t: Translate
  readonly owner?: string
  readonly batchId?: string
  readonly onShowAll?: () => void
}
const empty: ImageProfileInput = { name: '', baseUrl: '', model: '', providerType: 'openai_images', maxConcurrency: 3, timeoutSecs: 600 }

/**
 * 配置接口地址、模型和密钥，密钥不会读回表单。
 * @param props - 接口与翻译。
 * @returns 生图配置卡片。
 */
export function ImageSettings({ api, t }: ImageProps) {
  const [loading, setLoading] = useState(true)
  const [items, setItems] = useState<readonly ImageProfile[]>([])
  const [draft, setDraft] = useState<ImageProfileInput | null>(null)
  const [deleting, setDeleting] = useState<ImageProfile | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [version, setVersion] = useState(0)
  useEffect(() => {
    let active = true
    setLoading(true)
    api.profiles().then(
      (value) => { if (active) setItems(value) },
      (err: unknown) => { if (active) setError(messageOf(err)) },
    ).finally(() => { if (active) setLoading(false) })
    return () => { active = false }
  }, [api, version])
  async function action(run: () => Promise<void>): Promise<void> {
    setBusy(true); setError(''); setNotice('')
    try { await run(); setVersion(v => v + 1) } catch (err) { setError(messageOf(err)) } finally { setBusy(false) }
  }
  return <section className={css.page}>
    <div className={css.head}><h2>{t('image.settings')}</h2><Button onClick={() => { setDraft(empty); setError('') }}>{t('image.add')}</Button></div>
    <p className={css.muted}>{t('image.intro')}</p>
    {error && draft === null && <p role="alert" className={css.error}>{error}</p>}
    {notice && <p role="status">{notice}</p>}
    {loading && <p role="status">{t('image.loading')}</p>}
    {!loading && !error && items.length === 0 && <p className={css.muted}>{t('image.noProfiles')}</p>}
    {items.map(item => <div className={css.profile} key={item.id}>
      <div><strong>{item.name}</strong><p className={css.muted}>{item.model} · {item.baseUrl}</p>
        <p>{item.hasKey ? t('image.keySaved') : t('image.keyMissing')}{item.disabledReason ? ` · ${item.disabledReason}` : ''}</p></div>
      <div className={css.actions}>
        <Button disabled={busy} onClick={() => { setDraft({ id: item.id, name: item.name, baseUrl: item.baseUrl, model: item.model, providerType: item.providerType, maxConcurrency: item.maxConcurrency, timeoutSecs: item.timeoutSecs }) }}>{t('edit')}</Button>
        <Button disabled={busy} onClick={() => { void action(async () => { const result = await api.test(item.id); setNotice(result.supported ? t('image.testOk', { count: result.models.length }) : t('image.testUnsupported')) }) }}>{t('image.test')}</Button>
        <Button disabled={busy} onClick={() => { setDeleting(item) }}>{t('delete')}</Button>
      </div>
    </div>)}
    <Modal open={draft !== null} title={t('image.settings')} closeLabel={t('close')} onClose={() => { if (!busy) setDraft(null) }}
      footer={<Button disabled={busy || draft === null} onClick={() => { if (draft) void action(async () => { await api.save(draft); setDraft(null) }) }}>{busy ? t('saving') : t('save')}</Button>}>
      {draft && <div className={css.form}>
        <Field id="prism-image-name" label={t('image.name')}><Input id="prism-image-name" value={draft.name} onChange={(e) => { setDraft({ ...draft, name: e.target.value }) }} /></Field>
        <Field id="prism-image-url" label={t('image.url')}><Input id="prism-image-url" value={draft.baseUrl} placeholder={t('image.urlPlaceholder')} onChange={(e) => { setDraft({ ...draft, baseUrl: e.target.value }) }} /></Field>
        <Field id="prism-image-model" label={t('image.model')}><Input id="prism-image-model" value={draft.model} onChange={(e) => { setDraft({ ...draft, model: e.target.value }) }} /></Field>
        <Field id="prism-image-type" label={t('image.type')}><Picker id="prism-image-type" placeholder={t('image.type')} ariaLabel={t('image.type')} value={draft.providerType} options={[{ id: 'openai_images', label: t('image.openai') }, { id: 'async_task', label: t('image.async') }]} onChange={(value) => { setDraft({ ...draft, providerType: value as ImageProfileInput['providerType'] }) }} /></Field>
        <Field id="prism-image-key" label={t('image.key')}><Input id="prism-image-key" type="password" autoComplete="new-password" value={draft.apiKey ?? ''} placeholder={draft.id ? t('image.keepKey') : ''} onChange={(e) => { setDraft({ ...draft, apiKey: e.target.value }) }} /></Field>
        <Field id="prism-image-concurrency" label={t('image.concurrency')}><Input id="prism-image-concurrency" type="number" min={1} max={64} value={String(draft.maxConcurrency)} onChange={(e) => { setDraft({ ...draft, maxConcurrency: Number(e.target.value) }) }} /></Field>
        <Field id="prism-image-timeout" label={t('image.timeout')}><Input id="prism-image-timeout" type="number" min={10} max={1800} value={String(draft.timeoutSecs)} onChange={(e) => { setDraft({ ...draft, timeoutSecs: Number(e.target.value) }) }} /></Field>
        {error && <p role="alert" className={css.error}>{error}</p>}
      </div>}
    </Modal>
    <Modal open={deleting !== null} title={t('image.deleteTitle')} description={t('image.deleteBody')} closeLabel={t('close')}
      onClose={() => { if (!busy) setDeleting(null) }} footer={<Button disabled={busy} onClick={() => { if (deleting) void action(async () => { await api.remove(deleting.id); setDeleting(null) }) }}>{t('delete')}</Button>} />
  </section>
}

/**
 * 查询批次进度，并用素材路由显示仍保留的图片。
 * @param props - 接口、翻译和可选的会话筛选。
 * @returns 批次进度与图片结果。
 */
export function ImageResults({ api, t, owner, batchId, onShowAll }: ImageProps) {
  const [loading, setLoading] = useState(true)
  const [items, setItems] = useState<readonly ImageBatch[]>([])
  const [error, setError] = useState('')
  const [version, setVersion] = useState(0)
  const [busy, setBusy] = useState<string | null>(null)
  useEffect(() => {
    let active = true
    let timer: ReturnType<typeof setTimeout> | undefined
    const refresh = async (): Promise<void> => {
      try { const rows = batchId === undefined ? await api.batches() : [await api.batch(batchId)]
        if (active) { setItems(rows); setError('') } }
      catch (err) { if (active) setError(messageOf(err)) }
      finally { if (active) { setLoading(false); timer = setTimeout(() => { void refresh() }, 2000) } }
    }
    void refresh()
    return () => { active = false; clearTimeout(timer) }
  }, [api, version, batchId])
  const rows = owner === undefined ? items : items.filter(item => item.owner === owner)
  return <section className={css.page}>
    <div className={css.head}><h2>{t('image.results')}</h2><Button onClick={() => { setVersion(v => v + 1) }}>{t('retry')}</Button></div>
    {batchId && onShowAll && <Button onClick={onShowAll}>{t('image.showAll')}</Button>}
    <p className={css.muted}>{t('image.help')}</p>
    {error && <p role="alert" className={css.error}>{error}</p>}
    {loading && <p role="status">{t('image.loading')}</p>}
    {!loading && !error && rows.length === 0 && <p className={css.muted}>{t('image.empty')}</p>}
    {rows.map(batch => <article key={batch.id} className={css.batch}>
      <div className={css.head}><span>{t(`image.status.${batch.status}`)} · {batch.succeeded + batch.failed}/{batch.total}</span>
        {['pending', 'running', 'paused'].includes(batch.status) && <Button disabled={busy === batch.id} onClick={() => {
          setBusy(batch.id)
          api.cancel(batch.id).then(
            () => { setVersion(v => v + 1) },
            (err: unknown) => { setError(messageOf(err)) },
          ).finally(() => { setBusy(null) })
        }}>{t('image.cancel')}</Button>}</div>
      <p className={css.id}>{t('image.batchTitle', { seq: batch.seq })}</p>
      {batch.reason && <p role="status">{batch.reason}</p>}
      {[...new Set(batch.errors)].map(message => <p className={css.error} key={message}>{message}</p>)}
      <div className={css.grid}>{batch.assets.map(asset => asset.filePath && <a key={asset.id} href={fileUrl(asset.filePath)} target="_blank" rel="noreferrer">
        <img src={fileUrl(asset.thumbPath ?? asset.filePath)} alt={asset.title ?? t('image.result')} loading="lazy" />
        <span>{asset.title ?? t('image.result')}</span><small>{asset.id}</small>
      </a>)}</div>
    </article>)}
  </section>
}
