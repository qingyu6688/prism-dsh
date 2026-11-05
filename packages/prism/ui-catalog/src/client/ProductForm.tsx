/** 新建、编辑商品的对话框。 */
import clsx from 'clsx'
import { useEffect, useState, type ReactNode } from 'react'
import { Button, Input, Modal } from '@deepseek-ai/dsh-client-ui-primitives'
import type { Product } from '@prism/dsh-host-catalog/types'
import { messageOf, type CatalogApi } from './api.ts'
import { EMPTY_DRAFT, draftOf, inputOf, type DraftProblems, type ProductDraft } from './format.ts'
import type { CatalogKey } from './locales.ts'
import css from './Catalog.module.css'

/** 翻译函数。 */
export type Translate = (key: CatalogKey, params?: Record<string, string | number>) => string

export interface ProductFormProps {
  readonly open: boolean
  /** 编辑的商品；不传是新建。 */
  readonly product?: Product | undefined
  readonly api: CatalogApi
  readonly t: Translate
  readonly onClose: () => void
  /** 保存成功后调用，参数是保存后的商品。 */
  readonly onSaved: (product: Product) => void
}

const PROBLEM_TEXT = {
  required: 'form.required',
  cost: 'form.costInvalid',
  attribute: 'form.attributeInvalid',
} as const satisfies Record<NonNullable<DraftProblems[keyof DraftProblems]>, CatalogKey>

/**
 * 商品表单。名称必填，成本按元填，卖点和参数一行一条；货号重复等由服务端检查，错误显示在底部。
 * @param props - 是否打开、编辑的商品、接口和回调。
 * @returns 对话框。
 */
export function ProductForm({ open, product, api, t, onClose, onSaved }: ProductFormProps) {
  const [draft, setDraft] = useState<ProductDraft>(EMPTY_DRAFT)
  const [problems, setProblems] = useState<DraftProblems>({})
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  // 每次打开都从商品（或空表单）重新开始，不带上次没保存的内容
  useEffect(() => {
    if (!open) return
    setDraft(product === undefined ? EMPTY_DRAFT : draftOf(product))
    setProblems({})
    setError(null)
    setSaving(false)
  }, [open, product])

  const set = (field: keyof ProductDraft) => (value: string): void => {
    setDraft(current => ({ ...current, [field]: value }))
  }

  const submit = async (): Promise<void> => {
    const parsed = inputOf(draft)
    if ('problems' in parsed) {
      setProblems(parsed.problems)
      return
    }
    setProblems({})
    setError(null)
    setSaving(true)
    try {
      const saved = product === undefined
        ? await api.createProduct(parsed.input)
        : await api.updateProduct(product.id, parsed.input)
      onSaved(saved)
    } catch (failure) {
      setError(t('form.saveError', { message: messageOf(failure) }))
      setSaving(false)
    }
  }

  const problem = (field: keyof ProductDraft): string | undefined => {
    const key = problems[field]
    return key === undefined ? undefined : t(PROBLEM_TEXT[key])
  }

  return (
    <Modal
      open={open}
      onClose={onClose}
      closeLabel={t('close')}
      title={t(product === undefined ? 'form.createTitle' : 'form.editTitle')}
      className={clsx(css.formModal)}
      footer={<>
        {error === null ? null : <p className={css.formError} role="alert">{error}</p>}
        <Button variant="outline" onClick={onClose}>{t('cancel')}</Button>
        <Button variant="primary" disabled={saving} onClick={() => { void submit() }}>{t(saving ? 'saving' : 'save')}</Button>
      </>}
    >
      <form className={css.form} onSubmit={(event) => { event.preventDefault(); void submit() }}>
        <Field id="product-name" label={t('form.name')} required problem={problem('name')}>
          <Input id="product-name" value={draft.name} data-modal-autofocus onChange={(e) => { set('name')(e.target.value) }} />
        </Field>
        <div className={css.formRow}>
          <Field id="product-code" label={t('form.code')} hint={t('form.codeHint')}>
            <Input id="product-code" value={draft.code} onChange={(e) => { set('code')(e.target.value) }} />
          </Field>
          <Field id="product-category" label={t('form.category')}>
            <Input id="product-category" value={draft.category} onChange={(e) => { set('category')(e.target.value) }} />
          </Field>
        </div>
        <div className={css.formRow}>
          <Field id="product-brand" label={t('form.brand')}>
            <Input id="product-brand" value={draft.brand} onChange={(e) => { set('brand')(e.target.value) }} />
          </Field>
          <Field id="product-cost" label={t('form.cost')} problem={problem('cost')}>
            <Input id="product-cost" inputMode="decimal" value={draft.cost} onChange={(e) => { set('cost')(e.target.value) }} />
          </Field>
          <Field id="product-currency" label={t('form.currency')}>
            <Input id="product-currency" value={draft.currency} maxLength={3} onChange={(e) => { set('currency')(e.target.value) }} />
          </Field>
        </div>
        <Field id="product-markets" label={t('form.markets')} hint={t('form.marketsHint')}>
          <Input id="product-markets" value={draft.markets} onChange={(e) => { set('markets')(e.target.value) }} />
        </Field>
        <Field id="product-points" label={t('form.sellingPoints')} hint={t('form.sellingPointsHint')}>
          <textarea id="product-points" className={css.textarea} rows={3} value={draft.sellingPoints}
            onChange={(e) => { set('sellingPoints')(e.target.value) }} />
        </Field>
        <Field id="product-attributes" label={t('form.attributes')} hint={t('form.attributesHint')} problem={problem('attributes')}>
          <textarea id="product-attributes" className={css.textarea} rows={3} value={draft.attributes}
            onChange={(e) => { set('attributes')(e.target.value) }} />
        </Field>
        <Field id="product-description" label={t('form.description')}>
          <textarea id="product-description" className={css.textarea} rows={4} value={draft.description}
            onChange={(e) => { set('description')(e.target.value) }} />
        </Field>
        {/* 回车提交表单需要一个提交按钮；真正的按钮在对话框底部 */}
        <button type="submit" hidden />
      </form>
    </Modal>
  )
}

/** 表单的一项：标签、输入框、说明或错误。 */
export function Field({ id, label, required = false, hint, problem, children }: {
  readonly id: string
  readonly label: string
  readonly required?: boolean
  readonly hint?: string | undefined
  readonly problem?: string | undefined
  readonly children: ReactNode
}) {
  return (
    <div className={css.field}>
      <label className={css.fieldLabel} htmlFor={id}>
        {label}
        {required ? <span className={css.requiredMark} aria-hidden="true">*</span> : null}
      </label>
      {children}
      {problem === undefined
        ? (hint === undefined ? null : <span className={css.fieldHint}>{hint}</span>)
        : <span className={css.fieldProblem} role="alert">{problem}</span>}
    </div>
  )
}
