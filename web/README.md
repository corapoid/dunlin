# Dunlin website

The public marketing site for Dunlin. Static Astro build, light editorial
design, real product screenshots, no analytics and no cookies.

## Stack

- Astro 7 with static output
- React 19 islands for the nav and the hero showcase
- Tailwind CSS 4 through PostCSS
- Motion for entrance and tab transitions
- Phosphor Icons
- Manrope, Instrument Serif and Geist Mono, self hosted through Fontsource

## Development

Requires Node 22.23.4 or newer.

```sh
npm install
npm run dev
```

Open `http://localhost:4321`.

```sh
npm run check
npm run build
npm run preview
```

The production output is `dist/`.

## Structure

```text
src/components/     Nav, hero, bento, native, remote, FAQ, footer
src/layouts/        Base metadata shell and legal layout
src/pages/          /, /download, /privacy, /terms, 404, /og
src/assets/         Product screenshots processed by astro:assets
src/lib/site.ts     Links, version and download URLs
public/             Favicon, brand icon, OG image, robots.txt, licenses
```

## Content and links

- Site metadata, GitHub links, release version and download URLs live in
  `src/lib/site.ts`.
- Download links are pinned to `v0.1.0-beta.1`. Update them when a new release
  is published, or add stable asset aliases to the release workflow.
- The canonical domain is a placeholder (`https://dunlin.sh`). Override it with
  `SITE_URL` in the deploy environment, and update the sitemap URL in
  `public/robots.txt` when the final domain is chosen.
- Product screenshots live in `src/assets/screenshots/`. Replace them with new
  captures when the UI changes; Astro converts them to WebP at build time.

## Social card

`public/og.png` is generated from the hidden `/og/` page. It is a 1200×630
screenshot of that route. The route is excluded from the sitemap and blocked in
`robots.txt`.

To regenerate it, run the dev server, open `/og/`, and capture the 1200×630
viewport. Keep the route in sync with the current hero copy.

## Design notes

The site follows the `landing-page-design` rules from
[elayadesign/ai-design-skills](https://github.com/elayadesign/ai-design-skills)
with two deliberate choices:

- a light cream and clay palette instead of the default dark palette, to match
  the warm editorial reference;
- a serif display face (Instrument Serif) with Manrope for body copy, because
  the product is a native Mac app and the contrast reads better than a single
  sans face.

Motion uses the same fluid easing system: custom cubic beziers, blur and
translate reveals through `IntersectionObserver`, no scroll listeners, and a
full `prefers-reduced-motion` fallback. Backgrounds stay flat; there are no
decorative gradients except the hero headline and the product UI itself.

## Deploy

### Vercel

Import `corapoid/dunlin` and set the project root to `web`. The framework preset
is Astro; the build command is `npm run build` and the output directory is
`dist`. Set `SITE_URL` to the production URL.

### Cloudflare Pages

Use `web` as the root directory, `npm run build` as the build command and `dist`
as the output directory. Set `SITE_URL` to the production URL.

### GitHub Pages

Static output works, but GitHub Pages does not handle the clean URL fallback and
custom headers as well as Vercel or Cloudflare. Prefer one of those for the
public site.

## License

The site source is MIT, same as the app. Typeface licenses are in
`public/licenses/`. See the footer for the direct links.
