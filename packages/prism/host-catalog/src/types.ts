/**
 * 商品库、规则包、搜索的数据类型，照 Rust 的 `prism-catalog`、`prism-io`、`prism-rules` 结构写（驼峰字段）。
 * Rust 的 `Option` 序列化成 `null`，所以返回值里的可空字段写成 `T | null`。Rust 结构改了要同步改这里。
 * 方法表见 `docs/prism/service-api.md` 第 3.2 节。
 * @module @prism/dsh-host-catalog/types
 */

import type {} from '@deepseek-ai/dsh-typert-protocol'
import type { JsonValue } from '@deepseek-ai/dsh-util-values'

// ---------- 店铺 ----------

/** 新建、修改店铺时填的字段；没填的用默认值（语言 zh-CN、币种 CNY）。 */
export interface ShopInput {
  readonly name: string
  /** 平台标识，如 `amazon`、`temu`、`taobao`。 */
  readonly platform: string
  /** 站点，如 `US`；国内平台不填。 */
  readonly site?: string | null
  readonly language?: string
  readonly currency?: string
  readonly notes?: string | null
}

export interface Shop {
  readonly id: string
  readonly name: string
  readonly platform: string
  readonly site: string | null
  readonly language: string
  readonly currency: string
  readonly notes: string | null
  readonly sortOrder: number
  readonly createdAt: number
  readonly updatedAt: number
}

// ---------- 商品 ----------

/** 一项参数或规格，如 颜色 = 红。 */
export interface Attribute {
  readonly name: string
  readonly value: string
}

/** 新建、修改商品时填的字段。 */
export interface ProductInput {
  /** 货号，填了就必须唯一。 */
  readonly code?: string | null
  readonly name: string
  readonly category?: string | null
  readonly brand?: string | null
  readonly description?: string | null
  readonly sellingPoints?: readonly string[]
  readonly attributes?: readonly Attribute[]
  /** 成本（分）。 */
  readonly costCents?: number | null
  readonly currency?: string
  /** 目标市场，两位地区代码。 */
  readonly targetMarkets?: readonly string[]
}

export interface Product {
  readonly id: string
  readonly code: string | null
  readonly name: string
  readonly category: string | null
  readonly brand: string | null
  readonly description: string | null
  readonly sellingPoints: readonly string[]
  readonly attributes: readonly Attribute[]
  readonly costCents: number | null
  readonly currency: string
  readonly targetMarkets: readonly string[]
  readonly archived: boolean
  readonly createdAt: number
  readonly updatedAt: number
}

/** 商品列表的筛选条件。 */
export interface ProductFilter {
  /** 在名称、货号、类目、品牌里搜索。 */
  readonly keyword?: string | null
  /** `true` 只看已归档的，`false`（默认）只看在用的。 */
  readonly archived?: boolean
  /** 每页条数，0 或不填表示 50，最多 200。 */
  readonly limit?: number
  readonly offset?: number
}

export interface ProductPage {
  readonly items: readonly Product[]
  /** 同样条件下的总条数。 */
  readonly total: number
}

export interface Sku {
  readonly id: string
  readonly code: string | null
  /** 规格组合，如 颜色 = 红、尺码 = M。 */
  readonly spec: readonly Attribute[]
  readonly priceCents: number | null
  readonly costCents: number | null
  readonly stock: number | null
  readonly sortOrder: number
}

export interface ProductSummary {
  readonly skuCount: number
  readonly imageAssets: number
  readonly textAssets: number
  readonly starredAssets: number
  /** 各语言的文案条数。 */
  readonly copies: readonly { readonly language: string; readonly total: number; readonly approved: number }[]
  /** 各模块的批次数。 */
  readonly batches: readonly { readonly module: string; readonly count: number }[]
  readonly lastBatchAt: number | null
}

/** 商品详情：商品、规格、汇总。 */
export interface ProductDetail {
  readonly product: Product
  readonly skus: readonly Sku[]
  readonly summary: ProductSummary
}

// ---------- 从表格导入商品 ----------

/** 一个可导入的商品字段。 */
export interface FieldSpec {
  /** 字段键，如 `product.name`。 */
  readonly key: string
  /** 显示名，如"商品名称"。 */
  readonly label: string
  /** 表头的其他常见写法。 */
  readonly aliases: readonly string[]
  readonly required: boolean
}

/** 字段键 → 表格里的列名。没有出现的字段不导入。 */
export interface ImportMapping {
  readonly columns: Readonly<Record<string, string>>
}

/** 表格里的一行。单元格是文本、数字、布尔或空（日期已转成 `YYYY-MM-DD` 文本）。 */
export interface TableRow {
  /** 在表格里是第几行（从 1 开始）。 */
  readonly line: number
  readonly cells: readonly (string | number | boolean | null)[]
}

export interface ImportPreview {
  readonly sheetNames: readonly string[]
  readonly sheet: string
  readonly headerLine: number
  readonly headers: readonly string[]
  /** 前 5 行。 */
  readonly sample: readonly TableRow[]
  readonly totalRows: number
  readonly fields: readonly FieldSpec[]
  /** 建议的映射：套用保存过的，或者自动对列。 */
  readonly mapping: ImportMapping
  /** 套用了哪条保存过的映射；`overlap` 是表头重合度（0～1）。 */
  readonly matched: { readonly id: string; readonly name: string; readonly overlap: number } | null
}

export interface ImportPreviewRequest {
  /** Host 上的文件路径（界面先上传到 Host）。 */
  readonly path: string
  readonly sheet?: string | null
  readonly headerLine?: number | null
}

/** `createOnly`：货号重复的行跳过；`upsert`（默认）：货号相同时只改表格里有值的字段。 */
export type ImportMode = 'createOnly' | 'upsert'

export interface ImportProductsRequest {
  readonly path: string
  readonly sheet?: string | null
  readonly headerLine?: number | null
  readonly mapping: ImportMapping
  readonly mode?: ImportMode
  /** 不为空时把映射用这个名称存下来。 */
  readonly saveAs?: string | null
  /** 套用的是哪条保存过的映射，导入后记一次使用。 */
  readonly matchedMappingId?: string | null
}

export interface RowProblem {
  readonly line: number
  /** 字段显示名。 */
  readonly field: string
  readonly message: string
}

export interface ImportResult {
  readonly created: number
  readonly updated: number
  /** 有问题、没导入的行数。 */
  readonly skipped: number
  readonly problems: readonly RowProblem[]
  readonly savedMappingId: string | null
}

export interface SavedMapping {
  readonly id: string
  readonly name: string
  readonly headers: readonly string[]
  readonly mapping: ImportMapping
  readonly headerLine: number | null
  readonly useCount: number
  readonly lastUsedAt: number | null
}

// ---------- 素材 ----------

export interface Asset {
  readonly id: string
  /** `image` / `text`。 */
  readonly kind: string
  readonly productId: string | null
  /** `upload` / `generated` / `captured` / `imported`。 */
  readonly source: string
  readonly title: string | null
  /** 相对数据目录的路径，界面用 `/prism/files/<路径>` 显示。 */
  readonly filePath: string | null
  readonly thumbPath: string | null
  readonly textContent: string | null
  readonly mime: string | null
  readonly width: number | null
  readonly height: number | null
  readonly sha256: string | null
  readonly phash: string | null
  readonly originBatchId: string | null
  readonly originJobId: string | null
  readonly starred: boolean
  readonly tags: readonly string[]
  readonly createdAt: number
  readonly updatedAt: number
}

/** 素材列表的筛选条件。 */
export interface AssetFilter {
  readonly productId?: string | null
  readonly kind?: string | null
  readonly source?: string | null
  readonly tag?: string | null
  /** 在标题、文案内容、标签里搜索。 */
  readonly keyword?: string | null
  readonly starredOnly?: boolean
  readonly limit?: number
  readonly offset?: number
}

export interface AssetPage {
  readonly items: readonly Asset[]
  readonly total: number
}

export interface TagCount {
  readonly tag: string
  readonly count: number
}

export interface ImportAssetFilesRequest {
  /** Host 上的文件路径。 */
  readonly paths: readonly string[]
  readonly productId?: string | null
  readonly tags?: readonly string[]
}

export interface ImportAssetFilesResult {
  /** `deduplicated`：库里已有同样内容的图片，返回的是已有的那条。 */
  readonly imported: readonly { readonly asset: Asset; readonly deduplicated: boolean }[]
  /** 一个文件失败不影响其他文件。 */
  readonly failed: readonly { readonly path: string; readonly message: string }[]
}

// ---------- 规则包 ----------

/** 规则文件的类型。 */
export type RuleKind = 'rule_pack' | 'word_list' | 'export_template'

/** 多语言文本，键是语言代码，如 `zh-CN`、`en`。 */
export type LocalizedText = Readonly<Record<string, string>>

/** 规则从哪里来：随程序发布的内置版本，或用户导入的文件。 */
export type RuleOrigin = { readonly type: 'builtin' } | { readonly type: 'user'; readonly path: string }

/**
 * 规则包里设置页要显示的字段。完整内容（禁用词、各类文本和图片规则）见 `prism-rules` 的规则包格式，
 * 这里只列界面用到的；其余字段原样带回来，不做校验。
 */
export interface RulePackSummary {
  readonly id: string
  readonly revision: number
  readonly name: LocalizedText
  /** `draft` 未核对，`verified` 已核对。 */
  readonly status: string
  /** `cross_border` / `domestic`。 */
  readonly market: string
  readonly sites?: readonly string[]
  readonly notes?: string
  readonly text?: { readonly title?: { readonly maxLength: number; readonly unit: string } }
  readonly images?: {
    readonly main?: {
      readonly minSide?: number
      readonly maxBytes?: number
      readonly aspectRatios?: readonly string[]
      readonly whiteBackground?: string
    }
  }
}

/** 词表、导出模板在设置页只显示这些。 */
export interface RuleResourceSummary {
  readonly id: string
  readonly revision: number
  readonly name: LocalizedText
  readonly status: string
}

/** 一个生效的规则包和它的来源。 */
export interface RulePackRow {
  readonly origin: RuleOrigin
  /** 被它盖住的另一个版本的修订号。 */
  readonly shadowedRevision: number | null
  readonly value: RulePackSummary
}

/** 一个生效的词表或导出模板和它的来源。 */
export interface RuleResourceRow {
  readonly origin: RuleOrigin
  readonly shadowedRevision: number | null
  readonly value: RuleResourceSummary
}

/** 加载时有问题、被跳过的文件。 */
export interface RuleLoadProblem {
  readonly file: string
  readonly problems: readonly { readonly path: string; readonly message: string }[]
}

export interface RulesOverview {
  readonly packs: readonly RulePackRow[]
  readonly wordLists: readonly RuleResourceRow[]
  readonly exportTemplates: readonly RuleResourceRow[]
  readonly problems: readonly RuleLoadProblem[]
}

export interface ImportedRule {
  readonly kind: RuleKind
  readonly id: string
  readonly revision: number
}

// ---------- 搜索 ----------

export interface SearchResults {
  readonly products: readonly Product[]
  readonly assets: readonly Asset[]
}

// ---------- 通用 ----------

/** 按 id 操作一条记录。 */
export interface IdRequest {
  readonly id: string
}

/** 上传到 Host 的文件。 */
export interface UploadedFile {
  /** Host 上的完整路径，传给导入方法。 */
  readonly path: string
  readonly name: string
  readonly size: number
}

/** `prism/failed` 错误的附加信息：服务给的分类和细节（如规则文件的问题列表）。 */
export interface PrismFailureDetails {
  /** `not_found`、`invalid_input`、`storage`、`unavailable`、`timeout` 等，见 `docs/prism/service-api.md` 第 4 节。 */
  readonly code: string
  readonly detail: JsonValue
}

declare module '@deepseek-ai/dsh-typert-protocol' {
  interface RemoteErrorDetailsMap {
    /** 棱镜业务服务返回了错误，或者服务没在运行。放在类型文件里，浏览器端只引用类型也能拿到。 */
    'prism/failed': PrismFailureDetails
  }
}
