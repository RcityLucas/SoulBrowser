(() => {
  const config = __CONFIG__;
  const toText = (node) => (node && (node.textContent || '').replace(/\s+/g, ' ').trim()) || '';
  const selectors = Array.isArray(config.table_selectors) && config.table_selectors.length
    ? config.table_selectors
    : ['table'];
  const maxRows = typeof config.max_rows === 'number' && config.max_rows > 0 ? config.max_rows : 50;
  const tables = [];

  const extractTable = (node, selectorLabel) => {
    if (!node) return;
    const headers = [];
    const headerRow = node.querySelector('thead tr');
    if (headerRow) {
      headerRow.querySelectorAll('th').forEach((th) => {
        const text = toText(th);
        if (text) {
          headers.push(text);
        }
      });
    }
    const rows = [];
    node.querySelectorAll('tr').forEach((tr) => {
      const cells = [];
      tr.querySelectorAll('th, td').forEach((cell) => {
        const text = toText(cell);
        if (text) {
          cells.push(text);
        } else {
          cells.push('');
        }
      });
      if (cells.length > 0) {
        rows.push(cells);
      }
    });
    if (rows.length === 0) {
      return;
    }
    tables.push({
      selector: selectorLabel,
      headers,
      rows: rows.slice(0, maxRows),
    });
  };

  selectors.forEach((selector) => {
    try {
      document.querySelectorAll(selector).forEach((node, index) => {
        extractTable(node, `${selector}::${index}`);
      });
    } catch (err) {
      /* ignore invalid selectors */
    }
  });

  const keyValues = [];
  if (Array.isArray(config.key_value_selectors)) {
    config.key_value_selectors.forEach((rule) => {
      if (!rule || !rule.selector) {
        return;
      }
      let element;
      try {
        element = document.querySelector(rule.selector);
      } catch (_err) {
        element = null;
      }
      if (!element) {
        return;
      }
      let value = '';
      if (rule.attribute) {
        value = element.getAttribute(rule.attribute) || '';
      } else {
        value = toText(element);
      }
      const label = rule.label || toText(element) || rule.selector;
      if (value) {
        keyValues.push({
          label: label.slice(0, 80),
          value: value.slice(0, 200),
        });
      }
    });
  }

  const bodyText = (document.body?.innerText || '').replace(/\s+/g, ' ').trim();
  const textSample = bodyText.slice(0, 2000);

  return {
    ok: true,
    data: {
      kind: 'quote_observation',
      url: window.location.href,
      fetched_at: new Date().toISOString(),
      tables,
      key_values: keyValues,
      text_sample: textSample,
      text_sample_length: textSample.length,
      source: 'market.quote.fetch',
    },
  };
})();
