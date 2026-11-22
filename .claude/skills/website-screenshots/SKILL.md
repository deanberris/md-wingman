---
name: website-screenshots
description: Change the md-wingman.dev website in site/ and capture screenshots of it (light, dark, phone) or of the app for it.
---

# The website and its screenshots

The site is plain HTML and CSS in `site/` (`index.html`, `style.css`), published to <https://md-wingman.dev> by
`.github/workflows/pages.yml` on every push to `main` that touches `site/`. The custom domain lives in the repository's
Pages settings, not in a CNAME file. Keep the look minimalist and typographic, like the app.

## Preview screenshots of the site

Take them with Playwright (preinstalled in cloud containers: `require('playwright')`, `chromium.launch()`), against the
local files so nothing needs deploying:

```js
const { chromium } = require('playwright');
const url = 'file://' + process.cwd() + '/site/index.html';
(async () => {
  const browser = await chromium.launch();
  const shots = [
    ['md-wingman-site-light.png', { viewport: { width: 1280, height: 800 }, colorScheme: 'light' }],
    ['md-wingman-site-dark.png', { viewport: { width: 1280, height: 800 }, colorScheme: 'dark' }],
    ['md-wingman-site-phone.png', { viewport: { width: 390, height: 844 }, colorScheme: 'light', isMobile: true }],
  ];
  for (const [file, opts] of shots) {
    const page = await browser.newPage({ ...opts, deviceScaleFactor: 2 });
    await page.goto(url);
    await page.screenshot({ path: file, fullPage: true });
    await page.close();
  }
  await browser.close();
})();
```

Look at each image yourself before sharing: check both colour schemes, that nothing overflows at phone width, and that
links point at md-wingman.dev. In a project thread, save them under the project's `website-preview/` folder and attach
them to your reply; don't commit preview screenshots to the repository.

## Screenshots of the app for the site

These need the Mac (see `build-and-run`). Run the app with a sample post and `WINGMAN_FAKE=1`, so the sidebar shows
findings without calling Claude, and capture the window with `screencapture -l <window id>` or `screencapture -w`. Ask
Dean before taking over the screen on his iMac. Use sample text written for the purpose, never one of Dean's drafts.
Images committed for the site go in `site/` as optimised PNG or WebP, with `width`/`height` and `alt` set in the HTML.
