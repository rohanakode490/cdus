// @ts-check
import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';
import icon from 'astro-icon';

// https://astro.build/config
export default defineConfig({
  site: 'https://cdus.rohanakode.dev',
  trailingSlash: 'never',
  integrations: [sitemap(), icon()],
});
