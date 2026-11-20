for (const tablist of document.querySelectorAll('[role=tablist]')) {
  const tabs = Array.from(tablist.querySelectorAll('[role=tab]'));
  const panels = tabs.map(t => document.getElementById(t.dataset.panel));

  function selectTab(nextTab, focusTab) {
    tabs.forEach((tab, idx) => {
      const selected = tab === nextTab;
      tab.setAttribute('aria-selected', String(selected));
      tab.tabIndex = selected ? 0 : -1;
      const panel = panels[idx];
      if (!panel) return;
      if (selected) panel.removeAttribute('hidden');
      else panel.setAttribute('hidden', 'until-found');
    });
    if (focusTab) {
      nextTab.focus();
      nextTab.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    }
  }

  tabs.forEach((tab, idx) => {
    tab.addEventListener('click', () => selectTab(tab, false));
    tab.addEventListener('keydown', e => {
      const len = tabs.length;
      const next =
        e.key === 'ArrowRight' ? (idx + 1) % len :
        e.key === 'ArrowLeft' ? (idx - 1 + len) % len :
        e.key === 'Home' ? 0 :
        e.key === 'End' ? len - 1 : null;
      if (next === null) return;
      e.preventDefault();
      selectTab(tabs[next], true);
    });
    panels[idx]?.addEventListener('beforematch', () => selectTab(tab, false));
  });
}
