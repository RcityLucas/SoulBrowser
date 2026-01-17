(() => {
    const CONFIG = __CONFIG__;
    const rawSelectors = Array.isArray(CONFIG.selectors) ? CONFIG.selectors : [];
    const selectors = rawSelectors
        .map(value => (typeof value === 'string' ? value.trim() : ''))
        .filter(Boolean);
    const rawDomains = Array.isArray(CONFIG.domains) ? CONFIG.domains : [];
    const domains = rawDomains
        .map(value => (value || '').toString().toLowerCase().trim())
        .filter(Boolean);
    const markerAttr = CONFIG.marker_attr || 'data-soulbrowser-search-hit';
    const markerValue = CONFIG.marker_value || 'autoact-target';
    const maxCandidates = Number(CONFIG.max_candidates || 40);
    const excluded = new Set(
        Array.isArray(CONFIG.exclude_urls) ? CONFIG.exclude_urls.map(url => String(url || '').trim()) : [],
    );

    const cleanup = () => {
        try {
            document
                .querySelectorAll(`[${markerAttr}]`)
                .forEach(node => node.removeAttribute(markerAttr));
        } catch (err) {
            /* ignore */
        }
    };
    cleanup();

    const anchorFromNode = node => {
        if (!node) {
            return null;
        }
        if (node.tagName === 'A') {
            return node;
        }
        if (typeof node.closest === 'function') {
            const closest = node.closest('a[href]');
            if (closest) {
                return closest;
            }
        }
        if (typeof node.querySelector === 'function') {
            const nested = node.querySelector('a[href]');
            if (nested) {
                return nested;
            }
        }
        return null;
    };

    const decodeCandidate = anchor => {
        const sources = [
            anchor.getAttribute('data-landurl'),
            anchor.getAttribute('data-url'),
            anchor.getAttribute('data-href'),
            anchor.getAttribute('href'),
        ];
        for (const raw of sources) {
            const value = (raw || '').trim();
            if (!value || /^javascript:/i.test(value)) {
                continue;
            }
            try {
                const parsed = new URL(value, document.location.href);
                if (/baidu\.com/i.test(parsed.hostname)) {
                    const targetParam =
                        parsed.searchParams.get('url') || parsed.searchParams.get('target');
                    if (targetParam) {
                        try {
                            const decoded = decodeURIComponent(targetParam);
                            if (/^https?:/i.test(decoded)) {
                                return decoded;
                            }
                        } catch (err) {
                            if (/^https?:/i.test(targetParam)) {
                                return targetParam;
                            }
                        }
                    }
                }
                return parsed.href;
            } catch (err) {
                continue;
            }
        }
        return '';
    };

    const entries = [];
    const seen = new Set();
    const cap = Math.max(1, Math.min(maxCandidates, 80));

    selectors.forEach((selector, selectorIndex) => {
        if (!selector) {
            return;
        }
        let nodes;
        try {
            nodes = document.querySelectorAll(selector);
        } catch (err) {
            return;
        }
        Array.from(nodes).forEach((node, nodeIndex) => {
            if (entries.length >= cap) {
                return;
            }
            const anchor = anchorFromNode(node);
            if (!anchor) {
                return;
            }
            const url = decodeCandidate(anchor);
            if (!url) {
                return;
            }
            const normalized = url.split('#')[0];
            if (seen.has(normalized)) {
                return;
            }
            seen.add(normalized);
            entries.push({
                url,
                text: (anchor.innerText || '').trim(),
                node,
                selectorIndex,
                nodeIndex,
            });
        });
    });

    const hostOf = candidate => {
        try {
            return new URL(candidate).hostname.toLowerCase();
        } catch (err) {
            return '';
        }
    };

    const matchesDomain = (candidate, domain) => {
        const host = hostOf(candidate);
        if (!host) {
            return false;
        }
        const target = (domain || '').toLowerCase();
        if (!target) {
            return false;
        }
        return host === target || host.endsWith(`.${target}`);
    };

    const guardrailMatches = [];
    const fallbackEntries = [];
    for (const entry of entries) {
        if (excluded.has(entry.url)) {
            continue;
        }
        let matchedDomain = null;
        for (const domain of domains) {
            if (matchesDomain(entry.url, domain)) {
                matchedDomain = domain;
                break;
            }
        }
        if (matchedDomain) {
            guardrailMatches.push({ ...entry, matchedDomain });
        } else {
            fallbackEntries.push(entry);
        }
    }
    const prioritized = guardrailMatches.concat(fallbackEntries);
    const picked = prioritized[0] || null;

    if (!picked) {
        return { status: 'no_candidates', reason: '搜索结果未找到', candidate_count: 0 };
    }

    picked.node.setAttribute(markerAttr, markerValue);

    return {
        status: 'target_marked',
        anchor_selector: `[${markerAttr}="${markerValue}"]`,
        selected_url: picked.url,
        selected_text: picked.text,
        matched_domain: picked.matchedDomain || null,
        fallback_used: !picked.matchedDomain,
        candidate_count: prioritized.length,
        selectors_considered: selectors.length,
    };
})()
