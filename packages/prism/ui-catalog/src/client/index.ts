/**
 * 棱镜商品库界面，浏览器端：左侧导航的"商品库""素材库"页面、设置里的"店铺""规则包"、
 * 全局搜索框（Ctrl+Shift+K）、输入框 `@` 引用商品。
 *
 * 先挂上 `@prism/dsh-host-catalog` 生成的 Remote（`ctx.remote.prism`），再注册界面；Remote 和界面同生共死，
 * 不改 dsh 的 Remote 装配包。接口设计见 `docs/prism/design.md` 第 12 节。
 */
import { createElement } from 'react'
import type { Context } from '@deepseek-ai/cordis'
import type {} from '@deepseek-ai/dsh-api-remotes/client'
import type {} from '@deepseek-ai/dsh-client-locale/client'
import type { ShortcutCommandId } from '@deepseek-ai/dsh-client-shortcuts/client'
import type { UserTextNodeProps } from '@deepseek-ai/dsh-client-ui-conversation/client'
import type { InputTriggerServiceContract } from '@deepseek-ai/dsh-client-ui-input-trigger/client'
import type { MainPanelId } from '@deepseek-ai/dsh-client-ui-layout/client'
import type {} from '@deepseek-ai/dsh-client-ui-renderer/client'
import type {} from '@deepseek-ai/dsh-client-ui-settings/client'
import type {} from '@deepseek-ai/dsh-client-ui-sidebar/client'
import type { InjectFace, PropsLocale, PropsRuntime } from '@deepseek-ai/dsh-client-ui-slots'
import type { TypertRemoteContribution } from '@deepseek-ai/dsh-typert-protocol'
import remote from '@prism/dsh-host-catalog/remote'
import { createCatalogApi, type CatalogApi } from './api.ts'
import { AssetsPage } from './AssetsPage.tsx'
import { en, zh, type CatalogKey } from './locales.ts'
import { CatalogNavigator } from './navigator.ts'
import { createProductSource, ProductReference, productReferenceRule } from './product-reference.tsx'
import { AssetsIcon, ProductsIcon, RulesIcon, ShopIcon } from './ProductsIcon.tsx'
import { ProductsPage } from './ProductsPage.tsx'
import { RulesSection } from './RulesSection.tsx'
import { SearchPalette } from './SearchPalette.tsx'
import { ShopsSection } from './ShopsSection.tsx'

declare module '@deepseek-ai/dsh-client-ui-slots' {
  interface LocaleNamespaceMap { 'prism.catalog': CatalogKey }
}

/** 文案命名空间。 */
export const NS = 'prism.catalog'
/** 商品库页面在主区域的 id，也是左侧导航条目的 id。 */
export const PRODUCTS_PANEL = 'prism-products' as MainPanelId
/** 素材库页面在主区域的 id，也是左侧导航条目的 id。 */
export const ASSETS_PANEL = 'prism-assets' as MainPanelId
/** 全局搜索框打开时的快捷键作用域，和 `SearchPalette` 的 `shortcutModal` 一致。 */
const SEARCH_MODAL = 'prism-search'

/** 挂 Remote 之前就要有的服务。 */
export const inject = ['slots', 'locale', 'remote']

interface Injected {
  readonly api: CatalogApi
  readonly navigator: CatalogNavigator
}

type MainProps = PropsRuntime<'main'> & PropsLocale<'prism.catalog'> & InjectFace<Injected>
type SectionProps = PropsRuntime<'settings.section'> & PropsLocale<'prism.catalog'> & InjectFace<Injected>
type OverlayProps = PropsRuntime<'shell.overlay'> & PropsLocale<'prism.catalog'> & InjectFace<Injected>

function ProductsMain({ api, t, navigator }: MainProps) {
  return createElement(ProductsPage, { api, t, navigator })
}

function AssetsMain({ api, t, navigator }: MainProps) {
  return createElement(AssetsPage, { api, t, navigator })
}

function RulesMain({ api, t }: SectionProps) {
  return createElement(RulesSection, { api, t })
}

function ShopsMain({ api, t }: SectionProps) {
  return createElement(ShopsSection, { api, t })
}

function SearchOverlay({ api, t, navigator }: OverlayProps) {
  return createElement(SearchPalette, { api, t, navigator })
}

function ProductReferenceNode(props: UserTextNodeProps<'prism-product'>) {
  return createElement(ProductReference, props)
}

/**
 * 挂 Remote，再注册界面。
 * @param ctx - 浏览器端插件上下文。
 * @param contribution - 生成的 Remote 描述（测试时传替身）。
 * @returns 先撤界面、再卸 Remote 的清理函数。
 */
export async function mountCatalog(ctx: Context, contribution: TypertRemoteContribution): Promise<() => Promise<void>> {
  const unmount = await ctx.remote.$mount(contribution)
  const ui = ctx.inject(['remote.prism'], (inner) => { registerUi(inner) })
  try {
    await ui
  } catch (error) {
    await ui.dispose()
    await unmount()
    throw error
  }
  return async () => { await ui.dispose(); await unmount() }
}

/**
 * 注册文案、页面、导航条目、设置分区、搜索框和 `@` 引用。
 * 输入框、快捷键、主区域布局由别的插件提供，等它们就绪再注册，没有时相应功能不出现。
 * @param ctx - 已经有 `remote.prism` 的上下文。
 */
export function registerUi(ctx: Context): void {
  ctx.effect(() => ctx.locale.register(NS, { zh, en }), 'prism-catalog: locale')
  const t = ctx.locale.bind(NS)
  const api = createCatalogApi(ctx.remote.prism, t('unreachable'))
  const navigator = new CatalogNavigator(
    (panel) => { ctx.get('layout')?.selectPanel(panel as MainPanelId) },
    { products: PRODUCTS_PANEL, assets: ASSETS_PANEL },
  )
  const injected = (): Injected => ({ api, navigator })

  ctx.slots.inject('main', () => ctx.slots.register({
    name: 'main', key: PRODUCTS_PANEL, locale: NS, inject: injected,
  }, ProductsMain))
  ctx.slots.inject('sidebar.panellist', () => ctx.slots.register({
    name: 'sidebar.panellist', id: PRODUCTS_PANEL, order: 5, locale: NS, label: () => t('nav'),
  }, ProductsIcon))
  ctx.slots.inject('main', () => ctx.slots.register({
    name: 'main', key: ASSETS_PANEL, locale: NS, inject: injected,
  }, AssetsMain))
  ctx.slots.inject('sidebar.panellist', () => ctx.slots.register({
    name: 'sidebar.panellist', id: ASSETS_PANEL, order: 6, locale: NS, label: () => t('assets.nav'),
  }, AssetsIcon))
  ctx.slots.inject('settings.section', () => ctx.slots.register({
    name: 'settings.section', id: 'prism-shops', order: 30, locale: NS, label: () => t('shops.nav'), inject: injected,
  }, ShopsMain))
  ctx.slots.inject('settings.section.icon', () => ctx.slots.register({
    name: 'settings.section.icon', id: 'prism-shops',
  }, ShopIcon))
  ctx.slots.inject('settings.section', () => ctx.slots.register({
    name: 'settings.section', id: 'prism-rules', order: 31, locale: NS, label: () => t('rules.nav'), inject: injected,
  }, RulesMain))
  ctx.slots.inject('settings.section.icon', () => ctx.slots.register({
    name: 'settings.section.icon', id: 'prism-rules',
  }, RulesIcon))

  // 全局搜索框和它的快捷键
  ctx.slots.inject('shell.overlay', () => ctx.slots.register({
    name: 'shell.overlay', id: 'prism-search', locale: NS, inject: injected,
  }, SearchOverlay))
  ctx.inject(['shortcuts'], (inner) => {
    inner.effect(() => inner.shortcuts.register({
      id: 'prism.search' as ShortcutCommandId,
      label: () => t('search.open'),
      aliases: ['search products', 'search assets'],
      // Ctrl+K 是 dsh 的会话搜索；网页版浏览器本身占用了很多 Ctrl 组合键，和 dsh 一样再加 Alt
      defaults: {
        'desktop:macos': { code: 'KeyK', modifiers: ['primary', 'shift'] },
        'desktop:windows': { code: 'KeyK', modifiers: ['primary', 'shift'] },
        'desktop:linux': { code: 'KeyK', modifiers: ['primary', 'shift'] },
        'web:macos': { code: 'KeyK', modifiers: ['primary', 'alt', 'shift'] },
        'web:windows': { code: 'KeyK', modifiers: ['primary', 'alt', 'shift'] },
      },
      regions: ['page', 'editable'],
      modals: [SEARCH_MODAL],
      resolve: ({ modal }) => {
        if (modal !== null && modal !== SEARCH_MODAL) return { status: 'blocked', reason: 'modal' }
        return { status: 'handled', run: () => { if (modal === SEARCH_MODAL) navigator.closeSearch(); else navigator.openSearch() } }
      },
    }), 'prism-catalog: search shortcut')
  })

  // 输入框 `@` 引用商品，聊天记录里显示成标签
  ctx.inject(['inputTriggers'], (inner) => {
    const triggers = inner.get('inputTriggers') as InputTriggerServiceContract
    inner.effect(() => triggers.registerSource(createProductSource(api, t)), 'prism-catalog: @ products')
  })
  ctx.inject(['uiConversation'], (inner) => {
    inner.effect(() => inner.uiConversation.userText.register(productReferenceRule), 'prism-catalog: product mention rule')
  })
  ctx.slots.inject('conversation.user.text.node', () => ctx.slots.register({
    name: 'conversation.user.text.node', key: 'prism-product', priority: 100,
  }, ProductReferenceNode))
}

/**
 * 插件入口。
 * @param ctx - 浏览器端插件上下文。
 * @returns 清理函数。
 */
export async function apply(ctx: Context): Promise<() => Promise<void>> {
  return await mountCatalog(ctx, remote)
}
