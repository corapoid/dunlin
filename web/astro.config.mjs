import { defineConfig } from "astro/config";
import react from "@astrojs/react";
import sitemap from "@astrojs/sitemap";
import { fileURLToPath } from "node:url";

// The domain is a placeholder until the final domain is chosen. Set SITE_URL
// in the deploy environment to override canonical and social URLs.
export default defineConfig({
  site: process.env.SITE_URL ?? "https://dunlin.sh",
  devToolbar: { enabled: false },
  integrations: [
    react(),
    sitemap({ filter: (page) => !page.endsWith("/og/") }),
  ],
  vite: {
    resolve: {
      alias: {
        "@": fileURLToPath(new URL("./src", import.meta.url)),
      },
    },
  },
});
