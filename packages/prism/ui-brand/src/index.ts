/**
 * 棱镜界面品牌，Host 端。品牌只在浏览器端渲染（`exports["./client"]`），
 * 这里的空 apply 只是给 Loader 一个 Host 端条目。
 */

/** Host 端插件体：本包只提供浏览器端呈现。 */
export function apply(): void {}
