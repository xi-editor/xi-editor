---
layout: default
title: Home
permalink: /index.html
is_site_nav_category: true
site_nav_category: home
site_nav_category_order: 1
---

<div class="mdl-grid docs-content-wrapper mdl-grid--no-spacing">
  <div class="mdl-cell mdl-cell--6-col">
    <h1>Xi-Editor</h1>
    <p><em>(pronounced "Zigh")</em></p>

    <p>The xi editor project is an attempt to build a high quality text editor, using modern software engineering
    techniques. It is initially built for macOS, using Cocoa for the user interface. There are also frontends for
    other operating systems available from third-party developers.</p>

    <p>Goals include:</p>

    <ul>
    <li><p><strong><em>Incredibly high performance</em></strong>. All editing operations should commit and paint
    in under 16ms. The editor should never make you wait for anything.</p></li>

    <li><p><strong><em>Beauty</em></strong>. The editor should fit well on a modern desktop, and not look like a
    throwback from the ’80s or ’90s. Text drawing should be done with the best
    technology available (Core Text on Mac, DirectWrite on Windows, etc.), and
    support Unicode fully.</p></li>

    <li><p><strong><em>Reliability</em></strong>. Crashing, hanging, or losing work should never happen.</p></li>

    <li><p><strong><em>Developer friendliness</em></strong>. It should be easy to customize xi editor, whether
    by adding plug-ins or hacking on the core.</p></li>
    </ul>

    <p>Please refer to the <a href="https://github.com/xi-editor/xi-editor/issues/937">October 2018 roadmap</a>
    to learn more about planned features.</p>
  </div>

  <div class="mdl-cell mdl-cell--6-col">
      <img src="{{ site.baseurl }}/assets/home.png"/>
  </div>
</div>
