---
title: Built-in presets have moved
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

# Built-in presets have moved

[Browse the built-in preset directory](../built-in-presets/index.md) for the catalogue and individual workflows.

## Shell presets {#shell-presets}

[Shell presets](../built-in-presets/index.md#shell-presets)

## Image workflow {#image-workflow}

[Image workflow](../built-in-presets/image-tools.md)

## Application presets {#application-presets}

[Application presets](../built-in-presets/index.md#application-presets)

## System presets {#system-presets}

[System presets](../built-in-presets/index.md#system-presets)

## Ubuntu installation notes {#ubuntu-installation-notes}

[Ubuntu installation notes](../built-in-presets/system-bootstrap.md#ubuntu-installation-notes)

## Private split DNS {#private-split-dns}

[Private split DNS](../built-in-presets/split-dns.md)
