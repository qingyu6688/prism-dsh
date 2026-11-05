/** 棱镜品牌占用的界面位置：侧栏标志和名称、新会话首屏标志。替代 dsh-client-ui-brand-official。 */
import type { Context as ClientContext } from '@deepseek-ai/cordis'
import type {} from '@deepseek-ai/dsh-client-ui-renderer/client'
import type {} from '@deepseek-ai/dsh-client-ui-sidebar/client'
import type {} from '@deepseek-ai/dsh-client-ui-conversation/client'
import { HeroMark, SidebarMark, SidebarName } from './Brand.tsx'

/** 需要的服务：界面 slot 注册表。 */
export const inject = ['slots']

/**
 * 等各 slot 声明后再占用；声明消失（插件卸载、热更新）时一并撤回，不留半套品牌。
 * @param ctx - 浏览器端根上下文。
 */
export function apply(ctx: ClientContext): void {
  ctx.slots.inject('sidebar.brand.mark', () =>
    ctx.slots.inject('sidebar.brand.name', function* () {
      yield ctx.slots.register({ name: 'sidebar.brand.mark' }, SidebarMark)
      yield ctx.slots.register({ name: 'sidebar.brand.name' }, SidebarName)
    }))
  ctx.slots.inject('conversation.hero.brand.mark', () =>
    ctx.slots.register({ name: 'conversation.hero.brand.mark' }, HeroMark))
}
