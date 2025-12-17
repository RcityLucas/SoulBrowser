(() => {
  try {
    const textContent = (selector) => {
      const el = document.querySelector(selector);
      return el ? (el.textContent || '').trim().replace(/\s+/g, ' ') : null;
    };

    const attr = (selector, attrName) => {
      const el = document.querySelector(selector);
      return el ? el.getAttribute(attrName) : null;
    };

    const meta = (name) => {
      const el =
        document.querySelector(`meta[property="${name}"]`) ||
        document.querySelector(`meta[name="${name}"]`);
      return el ? el.getAttribute('content') : null;
    };

    const collectHeadings = () => {
      const seen = new Set();
      const headings = [];
      document.querySelectorAll('h1, h2, h3').forEach((node) => {
        const text = (node.textContent || '').trim().replace(/\s+/g, ' ');
        if (!text) {
          return;
        }
        const dedupe = `${node.tagName.toLowerCase()}::${text.toLowerCase()}`;
        if (seen.has(dedupe)) {
          return;
        }
        seen.add(dedupe);
        headings.push({ level: node.tagName.toLowerCase(), text });
      });
      return headings.slice(0, 12);
    };

    const collectParagraphs = () => {
      const out = [];
      const selectors = ['main p', 'article p', '.markdown-body p', '.prose p'];
      selectors.forEach((selector) => {
        document.querySelectorAll(selector).forEach((node) => {
          const text = (node.textContent || '').trim().replace(/\s+/g, ' ');
          if (text && text.length >= 24) {
            out.push(text);
          }
        });
      });
      return out.slice(0, 8);
    };

    const collectKeyValues = () => {
      const entries = [];
      const addEntry = (label, value) => {
        if (!label || !value) {
          return;
        }
        entries.push({
          label: label.trim().replace(/\s+/g, ' '),
          value: value.trim().replace(/\s+/g, ' '),
        });
      };

      document.querySelectorAll('dl').forEach((dl) => {
        const dts = dl.querySelectorAll('dt');
        const dds = dl.querySelectorAll('dd');
        for (let i = 0; i < Math.min(dts.length, dds.length); i += 1) {
          addEntry(dts[i].textContent || '', dds[i].textContent || '');
        }
      });

      document.querySelectorAll('table tr').forEach((row) => {
        const cells = row.querySelectorAll('th, td');
        if (cells.length >= 2) {
          addEntry(cells[0].textContent || '', cells[1].textContent || '');
        }
      });

      return entries.slice(0, 12);
    };

    const collectCounters = () => {
      const counters = [];
      const selectors = ['.Counter', '.social-count', '[data-view-component="true"].Counter'];
      selectors.forEach((selector) => {
        document.querySelectorAll(selector).forEach((node) => {
          const label = node.getAttribute('aria-label') || node.closest('a')?.textContent;
          const value = (node.textContent || '').trim();
          if (value) {
            counters.push({
              label: label ? label.trim().replace(/\s+/g, ' ') : null,
              value,
            });
          }
        });
      });
      return counters.slice(0, 12);
    };

    const collectLinks = () => {
      const links = [];
      const seen = new Set();
      document.querySelectorAll('main a[href], article a[href], nav a[href]').forEach((node) => {
        const href = node.getAttribute('href');
        if (!href || href.startsWith('#')) {
          return;
        }
        const absolute = (() => {
          try {
            return new URL(href, window.location.href).href;
          } catch (_err) {
            return href;
          }
        })();
        if (seen.has(absolute)) {
          return;
        }
        seen.add(absolute);
        const text = (node.textContent || '').trim().replace(/\s+/g, ' ');
        links.push({ text: text || null, url: absolute });
      });
      return links.slice(0, 15);
    };

    const collectIdentity = () => {
      const selectors = [
        'h1.vcard-names span.p-name',
        'h1 span[itemprop="name"]',
        '[data-test-selector="profile-name"]',
        '[data-test-selector="title"]',
        'header h1',
        'main h1',
        'h1',
      ];
      for (const selector of selectors) {
        const text = textContent(selector);
        if (text) {
          return text;
        }
      }
      return null;
    };

    const bodyText = (document.body?.innerText || '').replace(/\s+/g, ' ').trim();
    const text_sample = bodyText.slice(0, 4000);

    const headings = collectHeadings();
    const paragraphs = collectParagraphs();
    const key_values = collectKeyValues();
    const counters = collectCounters();
    const links = collectLinks();
    const identity = collectIdentity();

    const hero =
      textContent('main h1') ||
      textContent('main h2') ||
      textContent('header h1') ||
      textContent('header h2');

    const preview = {
      title: document.title || null,
      identity,
      hero,
      headline: headings.length > 0 ? headings[0].text : null,
      summary: paragraphs.length > 0 ? paragraphs[0] : null,
    };

    const data = {
      kind: 'generic_observation',
      url: window.location.href,
      title: document.title || null,
      identity,
      hero_text: hero,
      description: meta('description') || meta('og:description') || paragraphs[0] || null,
      meta: {
        og_title: meta('og:title'),
        og_type: meta('og:type'),
        site_name: meta('og:site_name'),
        twitter_title: meta('twitter:title'),
        twitter_handle: meta('twitter:site'),
        profile_username: meta('profile:username'),
        author: meta('author'),
      },
      headings,
      paragraphs,
      key_values,
      counters,
      links,
      text_sample,
      text_sample_length: text_sample.length,
      hero_image: attr('img[itemprop="image"]', 'src') || attr('img.avatar-user', 'src'),
      fetched_at: new Date().toISOString(),
      source: 'page.observe',
    };

    return { ok: true, data, preview };
  } catch (error) {
    return { ok: false, error: (error && error.message) || String(error) };
  }
})();
