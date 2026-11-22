---
title: 内置预设已迁移
unlisted: true
hide_table_of_contents: true
pagination_next: null
pagination_prev: null
---

import Head from '@docusaurus/Head';
import {useLocation} from '@docusaurus/router';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';

export function CanonicalLink() {
  const {pathname} = useLocation();
  const {siteConfig} = useDocusaurusContext();
  const target = pathname.replace(/\/reference\/built-in-presets$/, '/built-in-presets');
  return <Head><link rel="canonical" href={new URL(target, siteConfig.url).href} /></Head>;
}

<CanonicalLink />

# 内置预设已迁移

清单与专项说明现统一放在[内置预设目录](../built-in-presets/index.md)。

## Shell 预设 {#shell-预设}

[Shell 预设](../built-in-presets/index.md#shell-预设)

## 图片工作流 {#image-workflow}

[图片工作流](../built-in-presets/image-tools.md)

## App 预设 {#app-预设}

[App 预设](../built-in-presets/index.md#app-预设)

## 系统预设 {#系统预设}

[系统预设](../built-in-presets/index.md#系统预设)

## Ubuntu 安装说明 {#ubuntu-installation-notes}

[Ubuntu 安装说明](../built-in-presets/system-bootstrap.md#ubuntu-installation-notes)

## Private split DNS {#private-split-dns}

[Private split DNS](../built-in-presets/split-dns.md)
