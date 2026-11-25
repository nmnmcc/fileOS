// SPDX-License-Identifier: AGPL-3.0-only

import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";
import starlight from "@astrojs/starlight";

const site = process.env.DOCS_SITE_URL;
const defaultLocale = "en-us";
const locales = {
  "en-us": { label: "English", lang: "en-US" },
  "zh-cn": { label: "简体中文", lang: "zh-CN" },
};

export default defineConfig({
  site,
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
    }),
  ],
});
