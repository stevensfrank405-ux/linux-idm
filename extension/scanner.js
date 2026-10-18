function scanPage() {
      const found = new Map();
      for (const element of document.querySelectorAll('a[href],img[src],video,audio,source[src]')) {
        const url = element.href || element.currentSrc || element.src;
        if (!url || !/^https?:/i.test(url)) continue;
        found.set(url, { url, label: element.download || element.alt || element.textContent.trim().slice(0, 100) || url.split('/').pop() || url });
        if (found.size >= 200) break;
      }
      return [...found.values()];
    }
