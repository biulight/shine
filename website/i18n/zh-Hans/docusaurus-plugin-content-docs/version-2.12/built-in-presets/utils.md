---
title: Shell 小工具
sidebar_position: 7
---

# Shell 小工具

`utils` 预设提供导入 Shine 变量、同步终端主题和复制文件内容到剪贴板的命令。

## 安装

安装类别后打开新的 Shell：

```bash
shine shell install utils
```

使用安装后的命令名，不要直接运行预设目录中的脚本。

## 导出变量 {#export-a-value}

```bash
shine-env-export MY_TOKEN --as API_TOKEN
```

该命令将 Shine 明文值或解密后的 `<KEY>_SECRET` 导入当前 Shell。导出的值会留给后续命令使用；
只有一个程序需要时，使用[单次注入](../guides/environment.md#只向一个命令提供变量)。加密准备及
导出限制见[环境密钥](../guides/environment.md#使用-gpg-加密值)。

## 应用终端主题 {#terminal-theme}

```bash
shine-theme-sync
```

该命令将 `SHINE_TERMINAL_THEME` 和 `BAT_THEME` 应用到当前会话，无需额外 `eval`。检测行为、
profile 自动同步及 SSH 转发见[同步终端主题](../guides/terminal-theme-sync.md)。

## 复制文件到剪贴板

```bash
copyfile ./notes.txt
```

`copyfile` 通过 OSC 52 将文件内容发送到本地剪贴板。它仅适用于 Unix，终端或终端复用器必须允许 OSC 52。
