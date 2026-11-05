/**
 * 棱镜商品库界面，浏览器端：左侧导航的"商品库"页面、设置里的"店铺"分区。
 *
 * 先挂上 `@prism/dsh-host-catalog` 生成的 Remote（`ctx.remote.prism`），再注册界面；Remote 和界面同生共死，
 * 不改 dsh 的 Remote 装配包。接口设计见 `docs/prism/design.md` 第 12 节。
 */
import { createElement } from 'react'
import type { Context } from '@deepseek-ai/cordis'
import type {} from '@deepseek-ai/dsh-api-remotes/client'
import type {} from '@deepseek-ai/dsh-client-locale/client'
import type { MainPanelId } from '@deepseek-ai/dsh-client-ui-layout/client'
import type {} from '@deepseek-ai/dsh-client-ui-renderer/client'
import type {} from '@deepseek-ai/dsh-client-ui-settings/client'
import type {} from '@deepseek-ai/dsh-client-ui-sidebar/client'
import type { InjectFace, PropsLocale, PropsRuntime } from '@deepseek-ai/dsh-client-ui-slots'
import type { TypertRemoteContribution } from '@deepseek-ai/dsh-typert-protocol'
import remote from '@prism/dsh-host-catalog/remote'
import { createCatalogApi, type CatalogApi } from './api.ts'
import { en, zh, type CatalogKey } from './locales.ts'
import { AssetsPage } from './AssetsPage.tsx'
import { AssetsIcon, ProductsIcon, ShopIcon } from './ProductsIcon.tsx'
import { ProductsPage } from './ProductsPage.tsx'
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

/** 挂 Remote 之前就要有的服务。 */
export const inject = ['slots', 'locale', 'remote']

interface Injected {
  readonly api: CatalogApi
}

type MainProps = PropsRuntime<'main'> & PropsLocale<'prism.catalog'> & InjectFace<Injected>
type SectionProps = PropsRuntime<'settings.section'> & PropsLocale<'prism.catalog'> & InjectFace<Injected>

function ProductsMain({ api, t }: MainProps) {
  return createElement(ProductsPage, { api, t })
}

function AssetsMain({ api, t }: MainProps) {
  return createElement(AssetsPage, { api, t })
}

function ShopsMain({ api, t }: SectionProps) {
  return createElement(ShopsSection, { api, t })
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
 * 注册文案、页面、导航条目和设置分区。
 * @param ctx - 已经有 `remote.prism` 的上下文。
 */
export function registerUi(ctx: Context): void {
  ctx.effect(() => ctx.locale.register(NS, { zh, en }), 'prism-catalog: locale')
  const t = ctx.locale.bind(NS)
  const api = createCatalogApi(ctx.remote.prism, t('unreachable'))
  const injected = (): Injected => ({ api })

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
}

/**
 * 插件入口。
 * @param ctx - 浏览器端插件上下文。
 * @returns 清理函数。
 */
export async function apply(ctx: Context): Promise<() => Promise<void>> {
  return await mountCatalog(ctx, remote)
}
