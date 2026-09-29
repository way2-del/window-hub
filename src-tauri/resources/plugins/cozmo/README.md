# Bloub 表情（原 Cozmo 槽位）

基于开源项目 **[jeremy-prt/bloub](https://github.com/jeremy-prt/bloub)**（MIT）的 SVG 机器人表情引擎。

- 仓库：https://github.com/jeremy-prt/bloub
- 演示：https://bloub.vercel.app

本插件将 Bloub 的 `src/bot/` 引擎接入 Window Hub：快捷区常驻循环播放，弹窗里编排形状 / 颜色 / 默认表情与动画序列。

## 表面

| 表面 | 行为 |
|------|------|
| 快捷区 | 仅头像；播放已保存 montage（或内置情景）+ 当前外观 |
| 弹窗 | **外观 + 情景 + 序列编排**：改形状/颜色会立即同步到快捷区；「保存并应用」写入 montage |

## 编排模型

每块：`{ state, duration, expression? }`  
表情块 = `idle` + expression；动画块 = wink/orbit/…  

存储键：`appearance`（`shapeId` / `colorId` / `expressionId`）、`montage`、`scenes`、`activeSceneId`、`shortcutsCycle=custom`

> 说明：部分动画状态（如 sleep、orbit）在原项目中使用视频测量的专属轮廓（`baseBody=false`），此时会暂时盖过自定义形状；idle / wink / wide 等会显示你选的形状。

## 构建

`node plugins/cozmo/build.mjs`
