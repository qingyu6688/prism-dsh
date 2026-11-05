/** 设置里的"规则包"：列出生效的平台规则包、词表、导出模板，导入规则文件，删除导入的版本。 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { Button, IconPlusOutlineRegular, IconTrashOutlineRegular, Modal, Tag } from '@deepseek-ai/dsh-client-ui-primitives'
import type { LocalizedText, RuleKind, RulePackRow, RuleResourceRow, RulesOverview } from '@prism/dsh-host-catalog/types'
import { CatalogRequestError, messageOf, type CatalogApi } from './api.ts'
import type { CatalogKey } from './locales.ts'
import type { Translate } from './ProductForm.tsx'
import css from './Catalog.module.css'

const RULE_ACCEPT = '.json,.yaml,.yml'
const UNITS = ['char', 'byte', 'cjk_double']

export interface RulesSectionProps {
  readonly api: CatalogApi
  readonly t: Translate
}

type Load =
  | { readonly status: 'loading' }
  | { readonly status: 'ready'; readonly overview: RulesOverview }
  | { readonly status: 'error'; readonly message: string }

/** 导入的结果，显示在列表上方。 */
type Notice =
  | { readonly kind: 'ok'; readonly text: string }
  | { readonly kind: 'error'; readonly text: string; readonly problems: readonly { path: string; message: string }[] }

/** 要删除的导入版本。 */
interface Removal {
  readonly kind: RuleKind
  readonly id: string
  readonly name: string
}

/**
 * 多语言名称取界面语言的那个，没有就取英文，再没有取第一个。
 * @param name - 多语言文本。
 * @param locale - 界面语言对应的键，如 `zh-CN`。
 * @returns 显示的名称。
 */
export function localized(name: LocalizedText, locale: string): string {
  return name[locale] ?? name.en ?? Object.values(name)[0] ?? ''
}

/**
 * 规则包主图要求的一句话摘要，如"最短边 1000px · 比例 1:1 · 首图白底"。
 * @param row - 规则包。
 * @param t - 翻译函数。
 * @returns 摘要；没有主图要求时为短横线。
 */
export function imageSummary(row: RulePackRow, t: Translate): string {
  const main = row.value.images?.main
  if (main === undefined) return '—'
  const parts: string[] = []
  if (main.minSide !== undefined) parts.push(t('rules.minSide', { px: main.minSide }))
  if (main.aspectRatios !== undefined && main.aspectRatios.length > 0) parts.push(t('rules.ratios', { ratios: main.aspectRatios.join(' / ') }))
  if (main.whiteBackground === 'first') parts.push(t('rules.white.first'))
  if (main.whiteBackground === 'all') parts.push(t('rules.white.all'))
  return parts.length === 0 ? '—' : parts.join(' · ')
}

function titleLimit(row: RulePackRow, t: Translate): string {
  const title = row.value.text?.title
  if (title === undefined) return '—'
  return UNITS.includes(title.unit) ? t(`rules.unit.${title.unit}` as CatalogKey, { count: title.maxLength }) : `${title.maxLength} ${title.unit}`
}

/** 把服务端的问题列表整理成能显示的样子；格式不认识时不列。 */
function problemsOf(error: unknown): { path: string; message: string }[] {
  if (!(error instanceof CatalogRequestError) || !Array.isArray(error.detail)) return []
  return error.detail.flatMap((item: unknown) => {
    if (typeof item !== 'object' || item === null) return []
    const { path, message } = item as { path?: unknown; message?: unknown }
    return typeof message === 'string' ? [{ path: typeof path === 'string' ? path : '', message }] : []
  })
}

/**
 * 规则包分区。
 * @param props - 接口和文案。
 * @returns 设置分区内容。
 */
export function RulesSection({ api, t }: RulesSectionProps) {
  const [load, setLoad] = useState<Load>({ status: 'loading' })
  const [notice, setNotice] = useState<Notice | null>(null)
  const [importing, setImporting] = useState(false)
  const [removal, setRemoval] = useState<Removal | null>(null)
  const fileInput = useRef<HTMLInputElement | null>(null)
  const locale = t('rules.nameLocale')

  const refresh = useCallback(() => {
    api.listRules().then(
      (overview) => { setLoad({ status: 'ready', overview }) },
      (error: unknown) => { setLoad({ status: 'error', message: messageOf(error) }) },
    )
  }, [api])
  useEffect(() => { refresh() }, [refresh])

  const importFile = async (file: File | undefined): Promise<void> => {
    if (file === undefined) return
    setImporting(true)
    setNotice(null)
    try {
      const uploaded = await api.upload(file)
      const imported = await api.importRuleFile(uploaded.path)
      setNotice({ kind: 'ok', text: t('rules.imported', { id: imported.id, revision: imported.revision }) })
      refresh()
    } catch (error) {
      // 有问题列表时逐条列出，标题只说几处问题；服务给的整句说明里已经包含了所有问题，不再重复
      const problems = problemsOf(error)
      const text = problems.length > 0
        ? t('rules.importProblems', { count: problems.length })
        : t('rules.importError', { message: messageOf(error) })
      setNotice({ kind: 'error', text, problems })
    } finally {
      setImporting(false)
    }
  }

  const remove = async (target: Removal): Promise<void> => {
    setNotice(null)
    try {
      await api.removeUserRule(target.kind, target.id)
      refresh()
    } catch (error) {
      setNotice({ kind: 'error', text: t('shops.actionError', { message: messageOf(error) }), problems: [] })
    } finally {
      setRemoval(null)
    }
  }

  const origin = (row: RulePackRow | RuleResourceRow, kind: RuleKind) => (
    <span className={css.originCell}>
      <span>{t(row.origin.type === 'user' ? 'rules.origin.user' : 'rules.origin.builtin')}</span>
      {row.shadowedRevision === null ? null : <span className={css.productCode}>{t('rules.shadows', { revision: row.shadowedRevision })}</span>}
      {row.origin.type === 'user'
        ? <Button variant="ghost" size="sm" className={css.dangerButton} icon={<IconTrashOutlineRegular size={14} />}
          aria-label={`${t('rules.remove')} ${localized(row.value.name, locale)}`} title={t('rules.remove')}
          onClick={() => { setRemoval({ kind, id: row.value.id, name: localized(row.value.name, locale) }) }} />
        : null}
    </span>
  )

  const status = (value: string) => (
    <Tag tone={value === 'verified' ? 'success' : 'warning'}>{t(value === 'verified' ? 'rules.status.verified' : 'rules.status.draft')}</Tag>
  )

  const resources = (title: CatalogKey, rows: readonly RuleResourceRow[], kind: RuleKind) => (
    <section className={css.drawerSection}>
      <h3 className={css.sectionHead}>{t(title)}</h3>
      {rows.length === 0 ? <p className={css.muted}>{t('rules.empty')}</p> : (
        <table className={css.shopTable}>
          <thead><tr>
            <th>{t('rules.colName')}</th><th>{t('rules.colRevision')}</th><th>{t('rules.colStatus')}</th><th>{t('rules.colOrigin')}</th>
          </tr></thead>
          <tbody>{rows.map(row => (
            <tr key={row.value.id}>
              <td>{localized(row.value.name, locale)}</td>
              <td>{row.value.revision}</td>
              <td>{status(row.value.status)}</td>
              <td>{origin(row, kind)}</td>
            </tr>
          ))}</tbody>
        </table>
      )}
    </section>
  )

  return (
    <section className={css.settingsSection}>
      <div className={css.settingsHead}>
        <h2 className={css.settingsTitle}>{t('rules.nav')}</h2>
        <input ref={fileInput} type="file" accept={RULE_ACCEPT} hidden aria-label={t('rules.import')}
          onChange={(event) => { void importFile(event.target.files?.[0]); event.target.value = '' }} />
        <Button variant="primary" size="sm" icon={<IconPlusOutlineRegular size={14} />} disabled={importing}
          onClick={() => { fileInput.current?.click() }}>{t(importing ? 'rules.importing' : 'rules.import')}</Button>
      </div>
      <p className={css.muted}>{t('rules.intro')}</p>
      {notice === null ? null : (
        <div className={notice.kind === 'ok' ? css.successNote : css.stateBox} role={notice.kind === 'ok' ? 'status' : 'alert'}>
          <p>{notice.text}</p>
          {notice.kind === 'error' && notice.problems.length > 0
            ? <ul className={css.problemList}>{notice.problems.map(problem => (
              <li key={`${problem.path}-${problem.message}`}>
                {problem.path === '' ? null : <span className={css.problemLine}>{problem.path}</span>}
                <span>{problem.message}</span>
              </li>
            ))}</ul>
            : null}
        </div>
      )}
      {load.status === 'loading' ? <p className={css.muted}>{t('rules.loading')}</p> : null}
      {load.status === 'error'
        ? <div className={css.stateBox} role="alert">
          <p>{t('rules.loadError', { message: load.message })}</p>
          <Button variant="outline" size="sm" onClick={refresh}>{t('retry')}</Button>
        </div>
        : null}
      {load.status === 'ready' ? <>
        {load.overview.problems.length === 0 ? null : (
          <div className={css.notice} role="status">
            <p className={css.noticeTitle}>{t('rules.problemsTitle', { count: load.overview.problems.length })}</p>
            <ul className={css.problemList}>{load.overview.problems.map(problem => (
              <li key={problem.file}>
                <span className={css.problemLine}>{problem.file}</span>
                <span>{problem.problems.map(p => p.message).join('；')}</span>
              </li>
            ))}</ul>
          </div>
        )}
        <section className={css.drawerSection}>
          <h3 className={css.sectionHead}>{t('rules.packs')}</h3>
          <table className={css.shopTable}>
            <thead><tr>
              <th>{t('rules.colName')}</th><th>{t('rules.colRevision')}</th><th>{t('rules.colStatus')}</th>
              <th>{t('rules.colTitle')}</th><th>{t('rules.colImage')}</th><th>{t('rules.colOrigin')}</th>
            </tr></thead>
            <tbody>{load.overview.packs.map(row => (
              <tr key={row.value.id}>
                <td>{localized(row.value.name, locale)}</td>
                <td>{row.value.revision}</td>
                <td>{status(row.value.status)}</td>
                <td>{titleLimit(row, t)}</td>
                <td className={css.wrapCell}>{imageSummary(row, t)}</td>
                <td>{origin(row, 'rule_pack')}</td>
              </tr>
            ))}</tbody>
          </table>
        </section>
        {resources('rules.wordLists', load.overview.wordLists, 'word_list')}
        {resources('rules.templates', load.overview.exportTemplates, 'export_template')}
      </> : null}

      <Modal open={removal !== null} tone="warning" closeLabel={t('close')} onClose={() => { setRemoval(null) }}
        title={t('rules.removeTitle', { name: removal?.name ?? '' })} description={t('rules.removeBody')}
        footer={<>
          <Button variant="outline" onClick={() => { setRemoval(null) }}>{t('cancel')}</Button>
          <Button variant="primary" onClick={() => { if (removal !== null) void remove(removal) }}>{t('delete')}</Button>
        </>} />
    </section>
  )
}
