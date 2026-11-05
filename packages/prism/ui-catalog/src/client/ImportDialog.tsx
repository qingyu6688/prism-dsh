/**
 * 导入商品对话框，三步：选表格文件（上传到 Host）→ 对列（字段对应表格里的哪一列）→ 结果。
 * 对列方式可以保存，下次导入表头相似的表格时服务端自动套用。
 */
import clsx from 'clsx'
import { useEffect, useRef, useState } from 'react'
import { Button, Checkbox, Input, Modal, SegmentedControl } from '@deepseek-ai/dsh-client-ui-primitives'
import type { ImportMode, ImportPreview, ImportResult, UploadedFile } from '@prism/dsh-host-catalog/types'
import { messageOf, type CatalogApi } from './api.ts'
import { Picker } from './Picker.tsx'
import { Field, type Translate } from './ProductForm.tsx'
import css from './Catalog.module.css'

/** 结果里最多列出多少条问题行，再多就只说还有几条。 */
const MAX_LISTED_PROBLEMS = 50
/** 菜单里"不导入"这一项的 id，不会和表头重名（表头不会以 NUL 开头）。 */
const SKIP = '\u0000skip'
const TABLE_ACCEPT = '.xlsx,.xls,.ods,.csv'

type Step =
  | { readonly kind: 'pick' }
  | { readonly kind: 'busy'; readonly text: string }
  | { readonly kind: 'mapping'; readonly file: UploadedFile; readonly preview: ImportPreview }
  | { readonly kind: 'result'; readonly result: ImportResult }

export interface ImportDialogProps {
  readonly open: boolean
  readonly api: CatalogApi
  readonly t: Translate
  /** 关闭对话框；`imported` 为真时页面要刷新列表。 */
  readonly onClose: (imported: boolean) => void
}

/**
 * 导入商品对话框。
 * @param props - 是否打开、接口、文案和关闭回调。
 * @returns 对话框。
 */
export function ImportDialog({ open, api, t, onClose }: ImportDialogProps) {
  const [step, setStep] = useState<Step>({ kind: 'pick' })
  const [error, setError] = useState<string | null>(null)
  const [columns, setColumns] = useState<Record<string, string>>({})
  const [mode, setMode] = useState<ImportMode>('upsert')
  const [save, setSave] = useState(false)
  const [saveName, setSaveName] = useState('')
  const [headerLine, setHeaderLine] = useState('1')
  const fileInput = useRef<HTMLInputElement | null>(null)
  const imported = useRef(false)

  // 每次打开都从选文件开始
  useEffect(() => {
    if (!open) return
    setStep({ kind: 'pick' })
    setError(null)
    imported.current = false
  }, [open])

  const showPreview = (file: UploadedFile, preview: ImportPreview): void => {
    setColumns({ ...preview.mapping.columns })
    setHeaderLine(String(preview.headerLine))
    setMode('upsert')
    setSave(false)
    setSaveName(file.name.replace(/\.[^.]+$/, ''))
    setStep({ kind: 'mapping', file, preview })
  }

  const read = async (file: UploadedFile, sheet: string | null, line: number | null): Promise<void> => {
    setError(null)
    setStep({ kind: 'busy', text: t('import.reading') })
    try {
      showPreview(file, await api.previewProductImport({ path: file.path, sheet, headerLine: line }))
    } catch (failure) {
      setError(t('import.error', { message: messageOf(failure) }))
      setStep({ kind: 'pick' })
    }
  }

  const choose = async (picked: File | undefined): Promise<void> => {
    if (picked === undefined) return
    setError(null)
    setStep({ kind: 'busy', text: t('import.uploading', { name: picked.name }) })
    let uploaded: UploadedFile
    try {
      uploaded = await api.upload(picked)
    } catch (failure) {
      setError(t('import.error', { message: messageOf(failure) }))
      setStep({ kind: 'pick' })
      return
    }
    await read(uploaded, null, null)
  }

  const submit = async (file: UploadedFile, preview: ImportPreview): Promise<void> => {
    setError(null)
    setStep({ kind: 'busy', text: t('import.importing') })
    try {
      const result = await api.importProducts({
        path: file.path,
        sheet: preview.sheet === '' ? null : preview.sheet,
        headerLine: preview.headerLine,
        mapping: { columns },
        mode,
        saveAs: save && saveName.trim() !== '' ? saveName.trim() : null,
        matchedMappingId: preview.matched?.id ?? null,
      })
      imported.current = result.created + result.updated > 0
      setStep({ kind: 'result', result })
    } catch (failure) {
      setError(t('import.error', { message: messageOf(failure) }))
      setStep({ kind: 'mapping', file, preview })
    }
  }

  const close = (): void => { onClose(imported.current) }

  const footer = (() => {
    if (step.kind === 'mapping') {
      const missing = step.preview.fields.filter(field => field.required && columns[field.key] === undefined)
      return <>
        {error === null
          ? missing.length === 0 ? null : <p className={css.formError}>{t('import.requiredMissing', { fields: missing.map(f => f.label).join('、') })}</p>
          : <p className={css.formError} role="alert">{error}</p>}
        <Button variant="outline" onClick={() => { setStep({ kind: 'pick' }) }}>{t('import.back')}</Button>
        <Button variant="primary" disabled={missing.length > 0}
          onClick={() => { void submit(step.file, step.preview) }}>{t('import.submit', { rows: step.preview.totalRows })}</Button>
      </>
    }
    if (step.kind === 'result') return <Button variant="primary" onClick={close}>{t('import.done')}</Button>
    return <Button variant="outline" onClick={close}>{t('cancel')}</Button>
  })()

  return (
    <Modal open={open} onClose={close} closeLabel={t('close')} title={t('import.title')}
      className={clsx(css.formModal, css.wideModal)} footer={footer}>
      {step.kind === 'pick'
        ? <div className={css.dropZone}
          onDragOver={(event) => { event.preventDefault() }}
          onDrop={(event) => { event.preventDefault(); void choose(event.dataTransfer.files[0]) }}>
          <p className={css.muted}>{t('import.pickBody')}</p>
          <input ref={fileInput} type="file" accept={TABLE_ACCEPT} hidden aria-label={t('import.pickButton')}
            onChange={(event) => { void choose(event.target.files?.[0]); event.target.value = '' }} />
          <Button variant="primary" data-modal-autofocus onClick={() => { fileInput.current?.click() }}>{t('import.pickButton')}</Button>
          <p className={css.muted}>{t('import.dropHint')}</p>
          {error === null ? null : <p className={css.errorText} role="alert">{error}</p>}
        </div>
        : null}
      {step.kind === 'busy' ? <p className={css.stateText} aria-busy="true">{step.text}</p> : null}
      {step.kind === 'mapping'
        ? <MappingStep t={t} file={step.file} preview={step.preview} columns={columns} mode={mode} save={save} saveName={saveName}
          headerLine={headerLine} onHeaderLine={setHeaderLine} onColumns={setColumns} onMode={setMode}
          onSave={setSave} onSaveName={setSaveName}
          onReread={(sheet, line) => { void read(step.file, sheet, line) }} />
        : null}
      {step.kind === 'result' ? <ResultStep t={t} result={step.result} /> : null}
    </Modal>
  )
}

interface MappingStepProps {
  readonly t: Translate
  readonly file: UploadedFile
  readonly preview: ImportPreview
  readonly columns: Record<string, string>
  readonly mode: ImportMode
  readonly save: boolean
  readonly saveName: string
  readonly headerLine: string
  readonly onHeaderLine: (value: string) => void
  readonly onColumns: (next: Record<string, string>) => void
  readonly onMode: (mode: ImportMode) => void
  readonly onSave: (save: boolean) => void
  readonly onSaveName: (name: string) => void
  readonly onReread: (sheet: string | null, headerLine: number | null) => void
}

function MappingStep(props: MappingStepProps) {
  const { t, file, preview, columns } = props
  const headers = preview.headers.filter(header => header.trim() !== '')
  const options = [{ id: SKIP, label: t('import.skipColumn') }, ...headers.map(header => ({ id: header, label: header }))]
  const firstRow = preview.sample[0]
  const sample = (column: string | undefined): string => {
    if (column === undefined || firstRow === undefined) return ''
    const value = firstRow.cells[preview.headers.indexOf(column)]
    return value === null || value === undefined ? '' : String(value)
  }
  const line = Number.parseInt(props.headerLine, 10)

  return (
    <div className={css.form}>
      <p className={css.muted}>{t('import.file', { name: file.name, rows: preview.totalRows })}</p>
      <div className={css.formRow}>
        {preview.sheetNames.length > 1
          ? <Field id="import-sheet" label={t('import.sheet')}>
            <Picker id="import-sheet" value={preview.sheet} placeholder={t('import.sheet')}
              options={preview.sheetNames.map(name => ({ id: name, label: name }))}
              onChange={(sheet) => { props.onReread(sheet, null) }} />
          </Field>
          : null}
        <Field id="import-header-line" label={t('import.headerLine')}>
          <div className={css.inlineControls}>
            <Input id="import-header-line" className={clsx(css.lineInput)} inputMode="numeric" value={props.headerLine}
              onChange={(event) => { props.onHeaderLine(event.target.value) }} />
            <Button variant="outline" size="sm" disabled={!(line >= 1) || line === preview.headerLine}
              onClick={() => { props.onReread(preview.sheet === '' ? null : preview.sheet, line) }}>{t('import.reread')}</Button>
          </div>
        </Field>
      </div>
      {preview.matched === null ? null : <p className={css.successNote}>{t('import.matched', { name: preview.matched.name })}</p>}

      <table className={css.mappingTable}>
        <thead><tr><th>{t('import.colField')}</th><th>{t('import.colColumn')}</th><th>{t('import.colSample')}</th></tr></thead>
        <tbody>{preview.fields.map((field) => {
          const column = columns[field.key]
          return (
            <tr key={field.key}>
              <td>
                <label htmlFor={`map-${field.key}`}>{field.label}</label>
                {field.required ? <span className={css.requiredMark} aria-hidden="true">*</span> : null}
              </td>
              <td>
                <Picker id={`map-${field.key}`} value={column ?? SKIP} placeholder={t('import.skipColumn')} options={options}
                  onChange={(next) => {
                    // 选"不导入"就把这个字段从映射里去掉
                    const updated = Object.fromEntries(Object.entries(columns).filter(([key]) => key !== field.key))
                    props.onColumns(next === SKIP ? updated : { ...updated, [field.key]: next })
                  }} />
              </td>
              <td className={css.sampleCell}>{sample(column)}</td>
            </tr>
          )
        })}</tbody>
      </table>

      <Field id="import-mode" label={t('import.mode')}>
        <SegmentedControl id="import-mode" label={t('import.mode')} value={props.mode}
          options={[{ value: 'upsert', label: t('import.modeUpsert') }, { value: 'createOnly', label: t('import.modeCreate') }]}
          onChange={props.onMode} />
      </Field>
      {preview.matched === null
        ? <div className={css.saveMapping}>
          <Checkbox checked={props.save} onChange={props.onSave} label={t('import.saveMapping')} />
          {props.save
            ? <Input aria-label={t('import.mappingName')} value={props.saveName}
              onChange={(event) => { props.onSaveName(event.target.value) }} />
            : null}
        </div>
        : null}
    </div>
  )
}

function ResultStep({ t, result }: { readonly t: Translate; readonly result: ImportResult }) {
  const listed = result.problems.slice(0, MAX_LISTED_PROBLEMS)
  return (
    <div className={css.form}>
      <p className={css.resultLine}>
        <strong>{t('import.resultTitle')}</strong>
        <span>{t('import.created', { count: result.created })}</span>
        <span>{t('import.updated', { count: result.updated })}</span>
        <span>{t('import.skipped', { count: result.skipped })}</span>
      </p>
      {listed.length === 0 ? null : (
        <section className={css.drawerSection}>
          <h3 className={css.sectionHead}>{t('import.problems')}</h3>
          <ul className={css.problemList}>{listed.map(problem => (
            <li key={`${problem.line}-${problem.field}`}>
              <span className={css.problemLine}>{t('import.problemLine', { line: problem.line })}</span>
              {problem.field === '' ? null : <span className={css.muted}>{problem.field}</span>}
              <span>{problem.message}</span>
            </li>
          ))}</ul>
          {result.problems.length > listed.length
            ? <p className={css.muted}>{t('import.moreProblems', { count: result.problems.length - listed.length })}</p>
            : null}
        </section>
      )}
    </div>
  )
}
