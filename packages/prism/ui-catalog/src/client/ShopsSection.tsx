/** 设置里的"店铺"：列出店铺，添加、编辑、删除。 */
import clsx from 'clsx'
import { useCallback, useEffect, useState } from 'react'
import { Button, IconChevronDownOutlineRegular, IconEditOutlineRegular, IconPlusOutlineRegular, IconTrashOutlineRegular, Input, Menu, Modal } from '@deepseek-ai/dsh-client-ui-primitives'
import type { Shop, ShopInput } from '@prism/dsh-host-catalog/types'
import { messageOf, type CatalogApi } from './api.ts'
import { PLATFORMS, type CatalogKey } from './locales.ts'
import { Field, type Translate } from './ProductForm.tsx'
import css from './Catalog.module.css'

export interface ShopsSectionProps {
  readonly api: CatalogApi
  readonly t: Translate
}

type Load =
  | { readonly status: 'loading' }
  | { readonly status: 'ready'; readonly shops: readonly Shop[] }
  | { readonly status: 'error'; readonly message: string }

/**
 * 平台的显示名；不在列表里的平台标识原样显示。
 * @param platform - 平台标识。
 * @param t - 翻译函数。
 * @returns 显示名。
 */
export function platformLabel(platform: string, t: Translate): string {
  return (PLATFORMS as readonly string[]).includes(platform) ? t(`platform.${platform}` as CatalogKey) : platform
}

/**
 * 店铺分区。
 * @param props - 接口和文案。
 * @returns 设置分区内容。
 */
export function ShopsSection({ api, t }: ShopsSectionProps) {
  const [load, setLoad] = useState<Load>({ status: 'loading' })
  const [editing, setEditing] = useState<{ open: boolean; shop?: Shop | undefined }>({ open: false })
  const [deleting, setDeleting] = useState<Shop | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const refresh = useCallback(() => {
    api.listShops().then(
      (shops) => { setLoad({ status: 'ready', shops }) },
      (error: unknown) => { setLoad({ status: 'error', message: messageOf(error) }) },
    )
  }, [api])
  useEffect(() => { refresh() }, [refresh])

  const remove = async (shop: Shop): Promise<void> => {
    setActionError(null)
    try {
      await api.deleteShop(shop.id)
      refresh()
    } catch (error) {
      setActionError(t('shops.actionError', { message: messageOf(error) }))
    } finally {
      setDeleting(null)
    }
  }

  return (
    <section className={css.settingsSection}>
      <div className={css.settingsHead}>
        <h2 className={css.settingsTitle}>{t('shops.nav')}</h2>
        <Button variant="primary" size="sm" icon={<IconPlusOutlineRegular size={14} />}
          onClick={() => { setEditing({ open: true }) }}>{t('shops.new')}</Button>
      </div>
      <p className={css.muted}>{t('shops.intro')}</p>
      {actionError === null ? null : <p className={css.errorText} role="alert">{actionError}</p>}
      {load.status === 'loading' ? <p className={css.muted}>{t('shops.loading')}</p> : null}
      {load.status === 'error'
        ? <div className={css.stateBox} role="alert">
          <p>{t('shops.loadError', { message: load.message })}</p>
          <Button variant="outline" size="sm" onClick={refresh}>{t('retry')}</Button>
        </div>
        : null}
      {load.status === 'ready' && load.shops.length === 0 ? <p className={css.emptyLine}>{t('shops.empty')}</p> : null}
      {load.status === 'ready' && load.shops.length > 0
        ? <table className={css.shopTable}>
          <thead><tr>
            <th>{t('shops.colName')}</th><th>{t('shops.colPlatform')}</th><th>{t('shops.colSite')}</th>
            <th>{t('shops.colLanguage')}</th><th>{t('shops.colCurrency')}</th><th aria-label={t('edit')} />
          </tr></thead>
          <tbody>{load.shops.map(shop => (
            <tr key={shop.id}>
              <td>{shop.name}</td>
              <td>{platformLabel(shop.platform, t)}</td>
              <td>{shop.site ?? '—'}</td>
              <td>{shop.language}</td>
              <td>{shop.currency}</td>
              <td className={css.rowActions}>
                <Button variant="ghost" size="sm" aria-label={`${t('edit')} ${shop.name}`} icon={<IconEditOutlineRegular size={14} />}
                  onClick={() => { setEditing({ open: true, shop }) }} />
                <Button variant="ghost" size="sm" aria-label={`${t('delete')} ${shop.name}`} icon={<IconTrashOutlineRegular size={14} />}
                  className={css.dangerButton} onClick={() => { setDeleting(shop) }} />
              </td>
            </tr>
          ))}</tbody>
        </table>
        : null}

      <ShopForm open={editing.open} shop={editing.shop} api={api} t={t}
        onClose={() => { setEditing({ open: false }) }}
        onSaved={() => { setEditing({ open: false }); refresh() }} />
      <Modal
        open={deleting !== null}
        tone="warning"
        closeLabel={t('close')}
        onClose={() => { setDeleting(null) }}
        title={t('shops.deleteTitle', { name: deleting?.name ?? '' })}
        description={t('shops.deleteBody')}
        footer={<>
          <Button variant="outline" onClick={() => { setDeleting(null) }}>{t('cancel')}</Button>
          <Button variant="primary" onClick={() => { if (deleting !== null) void remove(deleting) }}>{t('delete')}</Button>
        </>}
      />
    </section>
  )
}

interface ShopDraft {
  name: string
  platform: string
  site: string
  language: string
  currency: string
  notes: string
}

function draftOf(shop: Shop | undefined): ShopDraft {
  return shop === undefined
    ? { name: '', platform: '', site: '', language: 'zh-CN', currency: 'CNY', notes: '' }
    : { name: shop.name, platform: shop.platform, site: shop.site ?? '', language: shop.language, currency: shop.currency, notes: shop.notes ?? '' }
}

/**
 * 店铺表单：名称、平台必填。
 * @param props - 是否打开、编辑的店铺、接口和回调。
 * @returns 对话框。
 */
function ShopForm({ open, shop, api, t, onClose, onSaved }: {
  readonly open: boolean
  readonly shop?: Shop | undefined
  readonly api: CatalogApi
  readonly t: Translate
  readonly onClose: () => void
  readonly onSaved: () => void
}) {
  const [draft, setDraft] = useState<ShopDraft>(draftOf(undefined))
  const [missing, setMissing] = useState<{ name?: boolean; platform?: boolean }>({})
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [pickerOpen, setPickerOpen] = useState(false)

  useEffect(() => {
    if (!open) return
    setDraft(draftOf(shop))
    setMissing({})
    setError(null)
    setSaving(false)
  }, [open, shop])

  const set = (field: keyof ShopDraft, value: string): void => { setDraft(current => ({ ...current, [field]: value })) }

  const submit = async (): Promise<void> => {
    const nextMissing = { name: draft.name.trim() === '', platform: draft.platform === '' }
    setMissing(nextMissing)
    if (nextMissing.name || nextMissing.platform) return
    const input: ShopInput = {
      name: draft.name.trim(),
      platform: draft.platform,
      site: draft.site.trim() === '' ? null : draft.site.trim().toUpperCase(),
      language: draft.language.trim() === '' ? 'zh-CN' : draft.language.trim(),
      currency: draft.currency.trim() === '' ? 'CNY' : draft.currency.trim().toUpperCase(),
      notes: draft.notes.trim() === '' ? null : draft.notes.trim(),
    }
    setSaving(true)
    setError(null)
    try {
      if (shop === undefined) await api.createShop(input)
      else await api.updateShop(shop.id, input)
      onSaved()
    } catch (failure) {
      setError(t('shops.saveError', { message: messageOf(failure) }))
      setSaving(false)
    }
  }

  return (
    <Modal
      open={open}
      onClose={onClose}
      closeLabel={t('close')}
      title={t(shop === undefined ? 'shops.createTitle' : 'shops.editTitle')}
      className={clsx(css.formModal)}
      footer={<>
        {error === null ? null : <p className={css.formError} role="alert">{error}</p>}
        <Button variant="outline" onClick={onClose}>{t('cancel')}</Button>
        <Button variant="primary" disabled={saving} onClick={() => { void submit() }}>{t(saving ? 'saving' : 'save')}</Button>
      </>}
    >
      <form className={css.form} onSubmit={(event) => { event.preventDefault(); void submit() }}>
        <Field id="shop-name" label={t('shops.name')} required problem={missing.name === true ? t('form.required') : undefined}>
          <Input id="shop-name" value={draft.name} data-modal-autofocus onChange={(e) => { set('name', e.target.value) }} />
        </Field>
        <Field id="shop-platform" label={t('shops.platform')} required problem={missing.platform === true ? t('form.required') : undefined}>
          <Menu
            open={pickerOpen}
            onClose={() => { setPickerOpen(false) }}
            portal
            minWidthFromAnchor
            selectedId={draft.platform === '' ? undefined : draft.platform}
            items={PLATFORMS.map(id => ({ id, label: platformLabel(id, t) }))}
            onSelect={(id: string) => { set('platform', id); setPickerOpen(false) }}
            anchor={
              <button type="button" id="shop-platform" className={css.picker} aria-haspopup="listbox" aria-expanded={pickerOpen}
                onClick={() => { setPickerOpen(value => !value) }}>
                <span className={draft.platform === '' ? css.pickerPlaceholder : undefined}>
                  {draft.platform === '' ? t('shops.platformPick') : platformLabel(draft.platform, t)}
                </span>
                <IconChevronDownOutlineRegular size={14} />
              </button>
            }
          />
        </Field>
        <div className={css.formRow}>
          <Field id="shop-site" label={t('shops.site')} hint={t('shops.siteHint')}>
            <Input id="shop-site" value={draft.site} onChange={(e) => { set('site', e.target.value) }} />
          </Field>
          <Field id="shop-language" label={t('shops.language')}>
            <Input id="shop-language" value={draft.language} onChange={(e) => { set('language', e.target.value) }} />
          </Field>
          <Field id="shop-currency" label={t('shops.currency')}>
            <Input id="shop-currency" value={draft.currency} maxLength={3} onChange={(e) => { set('currency', e.target.value) }} />
          </Field>
        </div>
        <Field id="shop-notes" label={t('shops.notes')}>
          <textarea id="shop-notes" className={css.textarea} rows={2} value={draft.notes} onChange={(e) => { set('notes', e.target.value) }} />
        </Field>
        <button type="submit" hidden />
      </form>
    </Modal>
  )
}
