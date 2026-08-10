// @ts-check

const {themes} = require('prism-react-renderer');

/** @type {import('@docusaurus/types').Config} */
const config = {
  title: 'Shine',
  tagline: 'Manage shell commands, application configuration, and system presets',
  url: 'https://biulight.github.io',
  baseUrl: '/shine/',
  organizationName: 'biulight',
  projectName: 'shine',
  trailingSlash: false,
  onBrokenLinks: 'throw',
  markdown: {
    hooks: {
      onBrokenMarkdownLinks: 'throw',
    },
  },
  i18n: {
    defaultLocale: 'en',
    locales: ['en', 'zh-Hans'],
    localeConfigs: {
      en: {
        label: 'English',
        htmlLang: 'en',
      },
      'zh-Hans': {
        label: '简体中文',
        htmlLang: 'zh-CN',
      },
    },
  },
  presets: [
    [
      'classic',
      /** @type {import('@docusaurus/preset-classic').Options} */ ({
        docs: {
          path: '../docs/manual',
          routeBasePath: '/',
          sidebarPath: require.resolve('./sidebars.js'),
          editUrl: ({locale, docPath}) => {
            const contentRoot =
              locale === 'en'
                ? 'docs/manual'
                : 'website/i18n/zh-Hans/docusaurus-plugin-content-docs/current';
            return `https://github.com/biulight/shine/edit/release/${contentRoot}/${docPath}`;
          },
          showLastUpdateAuthor: true,
          showLastUpdateTime: true,
        },
        blog: false,
        theme: {
          customCss: require.resolve('./src/css/custom.css'),
        },
      }),
    ],
  ],
  themeConfig:
    /** @type {import('@docusaurus/preset-classic').ThemeConfig} */ ({
      navbar: {
        title: 'Shine',
        items: [
          {
            type: 'docSidebar',
            sidebarId: 'manual',
            label: 'Documentation',
            position: 'left',
          },
          {
            type: 'localeDropdown',
            position: 'right',
          },
          {
            href: 'https://github.com/biulight/shine',
            label: 'GitHub',
            position: 'right',
          },
        ],
      },
      footer: {
        style: 'dark',
        links: [
          {
            title: 'Documentation',
            items: [
              {label: 'Installation', to: '/installation'},
              {label: 'Quick start', to: '/quick-start'},
              {label: 'Command reference', to: '/reference/commands'},
            ],
          },
          {
            title: 'Project',
            items: [
              {label: 'GitHub', href: 'https://github.com/biulight/shine'},
              {label: 'Releases', href: 'https://github.com/biulight/shine/releases'},
              {label: 'Report an issue', href: 'https://github.com/biulight/shine/issues'},
            ],
          },
          {
            title: 'Biulight',
            items: [
              {label: 'Knowledge base', href: 'https://blog.biulight.top/timeline/knowledge'},
            ],
          },
        ],
        copyright: `Copyright © ${new Date().getFullYear()} Biulight. Built with Docusaurus.`,
      },
      prism: {
        theme: themes.github,
        darkTheme: themes.dracula,
        additionalLanguages: ['powershell', 'toml'],
      },
    }),
};

module.exports = config;
