// SPDX-License-Identifier: AGPL-3.0-only

import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";
import starlight from "@astrojs/starlight";
import starlightSidebarTopics from "starlight-sidebar-topics";

// An empty value, as in a pull request build, is the same as no value.
const site = process.env.DOCS_SITE_URL || undefined;
// The path the site is served under, such as /homeserver on GitHub Pages.
const base = process.env.DOCS_BASE || undefined;
const defaultLocale = "en-us";
const locales = {
  "en-us": { label: "English", lang: "en-US" },
  "zh-cn": { label: "简体中文", lang: "zh-CN" },
};

// A topic label in every language, keyed by language tag.
const t = (en, zh) => ({ "en-US": en, "zh-CN": zh });

// One page in a sidebar: its slug (without the locale) and its labels.
const page = (slug, en, zh) => ({
  slug,
  label: en,
  translations: { "zh-CN": zh },
});

const group = (en, zh, items) => ({
  label: en,
  translations: { "zh-CN": zh },
  items,
});

// One manual per audience, switched at the top of the sidebar.
const topics = [
  {
    id: "user",
    label: t("User manual", "用户手册"),
    link: "/user/",
    icon: "seti:folder",
    items: [
      page("user", "Overview", "概览"),
      page("user/getting-started", "Getting started", "开始使用"),
      group("Your data", "你的数据", [
        page("user/data", "Spaces and items", "空间与条目"),
        page("user/finding", "Finding things", "查找"),
        page("user/sharing", "Sharing", "分享"),
      ]),
      page("user/apps", "Apps and assistants", "应用与助手"),
      page("user/recovery", "Your recovery code", "恢复码"),
    ],
  },
  {
    id: "admin",
    label: t("Administration manual", "管理手册"),
    link: "/admin/",
    icon: "setting",
    items: [
      page("admin", "Overview", "概览"),
      page("admin/install", "Installation", "安装"),
      page("admin/running", "Running a node", "运行节点"),
      page("admin/configuration", "Configuration", "配置"),
      page("admin/access", "Tokens and grants", "令牌与授权"),
      page("admin/backups", "Backups and restore", "备份与恢复"),
      page("admin/sign-in", "Sign-in for other apps", "为其他应用提供登录"),
      page("admin/troubleshooting", "Troubleshooting", "故障排除"),
    ],
  },
  {
    id: "dev",
    label: t("Developer manual", "开发者手册"),
    link: "/dev/",
    icon: "seti:rust",
    items: [
      page("dev", "Overview", "概览"),
      page("dev/contributing", "Contributing", "参与贡献"),
      page("dev/api", "REST API", "REST API"),
      page("dev/architecture", "Architecture", "架构"),
      page("dev/fileos", "Design: fileOS", "设计：fileOS"),
      page("dev/prior-art", "Projects to learn from", "值得学习的项目"),
    ],
  },
];

export default defineConfig({
  site,
  base,
  integrations: [
    sitemap({
      filter: (url) => new URL(url).pathname !== "/",
      i18n: {
        defaultLocale,
        locales: Object.fromEntries(
          Object.entries(locales).map(([path, { lang }]) => [path, lang]),
        ),
      },
    }),
    starlight({
      title: "homeserver",
      defaultLocale,
      locales,
      plugins: [
        // The landing page of each language introduces all three manuals.
        starlightSidebarTopics(topics, { exclude: ["/en-us", "/zh-cn"] }),
      ],
    }),
  ],
});
