---
layout: page
title: Building Docs
site_nav_category: buildingdocs
site_nav_category_order: 500
is_site_nav_category: true
---

## Development

You can run the site locally on your computer while making changes.

### Setup Ruby and Bundler

Ensure that you have Ruby and [Bundler](http://bundler.io/) installed.

```
gem install bundler
```

### One-time setup

```
bundle install --path vendor/bundle
```

_Note: If you're on Mac OS and this fails installing nokogiri, run `brew unlink xz`, install, and then `brew link xz`._

### Running the site

```
bundle exec jekyll serve
```

Point your browser at [http://127.0.0.1:4000/xi-editor/](http://127.0.0.1:4000/xi-editor/).
