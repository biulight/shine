---
title: 图片工具
sidebar_position: 4
---

# 图片工具

`image-tools` 提供 JPEG、PNG、WebP 的批量压缩、缩放和格式转换，保留原图。
压缩后的文件不保证更小。

## 安装与使用

运行命令的每台机器都必须在 `PATH` 中安装 Bun 1.3.14 或更高版本；Shine 不内置 Bun，
无需另外安装图片处理库。

查看并安装类别：

```bash
shine info shell/image-tools
shine install shell/image-tools
```

只需要压缩时，可改为安装 `shell/image-tools/img-compress`。安装后打开新终端或重新加载
Shell 配置，按需选择操作：

| 操作 | 示例 |
| --- | --- |
| 压缩一个文件及目录中的图片 | `img-compress photo.jpg screenshots/` |
| 将宽度限制为 1280，并另存到指定目录 | `img-resize --width 1280 --output-dir ./resized photos/` |
| 以品质 75 转换为 WebP | `img-convert --format webp --quality 75 --output-dir ./webp hero.png gallery/` |

更新与卸载方法见[管理 Shell 预设](../guides/shell-presets.md)。

每个命令都接受多个文件或目录输入。目录只扫描第一层的 JPEG、PNG、WebP，不会递归。未指定
`--output-dir` 时，结果保存在来源旁边，名称形如 `photo.compressed.jpg`、
`photo.resized.jpg` 或所选转换格式的扩展名。指定输出目录后，所有结果平铺到该目录；重复目标名会
明确失败。已有目标也会失败，只有 `--force` 才允许替换；来源图片永远不会被原地修改。
`--force` 也不允许替换输入图片。

批处理遇到单项失败后会继续，并在存在任意失败时返回非零状态。前 20 条失败会显示在终端；超过
20 条时，完整列表还会写入 `--output-dir` 下唯一命名的 `image-tools-errors-*.log`，未指定输出目录
时则写入当前目录。

`IMAGE_QUALITY`、`IMAGE_MAX_WIDTH`、`IMAGE_MAX_HEIGHT` 默认分别为 `80`、`1920`、`1080`。
品质设置适用于 JPEG/WebP，PNG 使用固定压缩级别。用 `--quality`、`--width`、`--height`
覆盖当次运行，或用 `shine env set` 保留本机默认值，
详见[环境设置](../guides/environment.md#查看和设置变量)。默认 snapshot 模式下，修改
复制或外部来源后仍需执行 `shine upgrade`，已安装命令才会变化。
