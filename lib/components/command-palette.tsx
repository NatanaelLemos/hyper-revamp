import React, {useCallback, useEffect, useMemo, useRef, useState} from 'react';

import {sendSessionData} from '../actions/sessions';
import {requestTermGroup} from '../actions/term-groups';

type SlashCommand = {
  name: string;
  description: string;
  /** Dynamic sub-items resolved at open time (e.g. list of profiles). */
  getArgs?: () => Array<{value: string; description?: string}>;
  run: (arg?: string) => void;
};

export const CommandPalette: React.FC = () => {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('/');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const navigatedRef = useRef(false);
  const stateRef = useRef({open: false, query: '/', selectedIndex: 0, itemCount: 0});
  // Approximate column position on the current shell line. We can't read the
  // shell's state directly, so we track keystrokes since the last Enter/clear.
  // 0 means we believe the cursor is at the start of a fresh prompt line.
  const lineColRef = useRef(0);

  const close = useCallback(() => {
    setOpen(false);
    setQuery('/');
    setSelectedIndex(0);
    navigatedRef.current = false;
  }, []);

  // Keep a ref copy so the global keydown handler (attached once) sees fresh values.
  useEffect(() => {
    stateRef.current.open = open;
    stateRef.current.query = query;
    stateRef.current.selectedIndex = selectedIndex;
  }, [open, query, selectedIndex]);

  const commands = useMemo<SlashCommand[]>(() => {
    const getProfiles = (): Array<{name: string}> => {
      const store = (window as any).store;
      const s = store?.getState?.();
      const p = s?.ui?.profiles;
      if (p?.asMutable) return p.asMutable({deep: true});
      return p || [];
    };
    return [
      {
        name: '/profile',
        description: 'Open a new tab in a specific profile',
        getArgs: () => getProfiles().map((p) => ({value: p.name})),
        run: (arg) => {
          if (!arg) return;
          (window as any).store?.dispatch?.(requestTermGroup(undefined, arg));
        }
      }
    ];
  }, []);

  const items = useMemo(() => {
    const q = query.toLowerCase();
    const results: Array<{label: string; hint?: string; onRun: () => void}> = [];
    for (const cmd of commands) {
      if (cmd.getArgs) {
        const args = cmd.getArgs();
        // If query is just the command name (no space yet), still show all args.
        for (const a of args) {
          const label = `${cmd.name} ${a.value}`;
          if (!q || q === '/' || label.toLowerCase().startsWith(q) || cmd.name.startsWith(q)) {
            results.push({label, hint: a.description || cmd.description, onRun: () => cmd.run(a.value)});
          }
        }
      } else if (!q || q === '/' || cmd.name.toLowerCase().startsWith(q)) {
        results.push({label: cmd.name, hint: cmd.description, onRun: () => cmd.run()});
      }
    }
    return results;
  }, [commands, query]);

  useEffect(() => {
    stateRef.current.itemCount = items.length;
    if (selectedIndex >= items.length) setSelectedIndex(0);
  }, [items.length, selectedIndex]);

  // Global keydown — capture phase so we can intercept arrow keys before xterm.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Don't engage while a real form input is focused. xterm also uses a
      // hidden textarea for input — treat that as "terminal", not "form".
      const target = e.target as HTMLElement | null;
      if (target) {
        const isFormInput = /^(INPUT|SELECT)$/.test(target.tagName) ||
          (target.tagName === 'TEXTAREA' && !target.classList.contains('xterm-helper-textarea'));
        if (isFormInput) return;
      }

      const s = stateRef.current;

      // Start the palette when the user types `/` plain at the start of a line.
      if (!s.open) {
        if (
          e.key === '/' &&
          !e.metaKey &&
          !e.ctrlKey &&
          !e.altKey &&
          lineColRef.current === 0
        ) {
          // Don't preventDefault — let the shell receive the `/` so it appears
          // on the prompt line as the user types.
          setOpen(true);
          setQuery('/');
          setSelectedIndex(0);
          navigatedRef.current = false;
          lineColRef.current += 1;
          return;
        }
        // Track line position for the non-palette case.
        if (e.key === 'Enter') {
          lineColRef.current = 0;
        } else if (e.key === 'Backspace') {
          lineColRef.current = Math.max(0, lineColRef.current - 1);
        } else if ((e.ctrlKey || e.metaKey) && (e.key === 'u' || e.key === 'c' || e.key === 'a' || e.key === 'l')) {
          // Ctrl+U (clear line), Ctrl+C (abort), Ctrl+A (beginning of line), Ctrl+L (clear screen).
          lineColRef.current = 0;
        } else if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
          lineColRef.current += 1;
        }
        return;
      }

      // Palette is open. Handle navigation / submission.
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        e.stopPropagation();
        navigatedRef.current = true;
        setSelectedIndex((i) => Math.min(i + 1, Math.max(s.itemCount - 1, 0)));
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        e.stopPropagation();
        navigatedRef.current = true;
        setSelectedIndex((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === 'Tab') {
        e.preventDefault();
        e.stopPropagation();
        navigatedRef.current = true;
        setSelectedIndex((i) => (s.itemCount ? (i + 1) % s.itemCount : 0));
        return;
      }
      if (e.key === 'Escape') {
        close();
        return;
      }
      if (e.key === 'Enter') {
        lineColRef.current = 0;
        // Run the selected command only if the user navigated OR the current
        // query unambiguously matches an item. Otherwise let Enter go to shell.
        if (navigatedRef.current && s.itemCount > 0) {
          e.preventDefault();
          e.stopPropagation();
          const item = items[s.selectedIndex];
          if (item) {
            // Wipe the `/xyz` characters the user typed on the shell prompt.
            (window as any).store?.dispatch?.(sendSessionData(null, '\x15'));
            item.onRun();
          }
          close();
        } else {
          close();
        }
        return;
      }
      if (e.key === 'Backspace') {
        lineColRef.current = Math.max(0, lineColRef.current - 1);
        if (s.query.length <= 1) {
          // Backspacing past the leading `/` → close the palette.
          close();
        } else {
          setQuery((q) => q.slice(0, -1));
          setSelectedIndex(0);
          navigatedRef.current = false;
        }
        return;
      }
      // Printable character — append to local query. Don't intercept; shell
      // still receives it so the user sees what they typed in the terminal.
      if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
        setQuery((q) => q + e.key);
        setSelectedIndex(0);
        navigatedRef.current = false;
        lineColRef.current += 1;
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [items, close]);

  if (!open) return null;

  return (
    <div className="cmdp_inline">
      <div className="cmdp_inline_head">
        <span className="cmdp_query">{query}</span>
        <span className="cmdp_hint_kbd">↑↓ navigate · Enter select · Esc dismiss</span>
      </div>
      <div className="cmdp_list">
        {items.length === 0 ? (
          <div className="cmdp_empty">No matching commands</div>
        ) : (
          items.map((it, i) => (
            <div
              key={it.label}
              className={`cmdp_item ${i === selectedIndex ? 'selected' : ''}`}
              onMouseEnter={() => setSelectedIndex(i)}
              onMouseDown={(e) => {
                e.preventDefault();
                (window as any).store?.dispatch?.(sendSessionData(null, '\x15'));
                it.onRun();
                close();
              }}
            >
              <span className="cmdp_label">{it.label}</span>
              {it.hint ? <span className="cmdp_hint">{it.hint}</span> : null}
            </div>
          ))
        )}
      </div>
      <style jsx>{`
        .cmdp_inline {
          position: fixed;
          left: 16px;
          bottom: 16px;
          width: min(520px, calc(100vw - 32px));
          z-index: 1000;
          background: rgba(20, 22, 28, 0.97);
          border: 1px solid rgba(255, 255, 255, 0.12);
          border-radius: 8px;
          box-shadow: 0 8px 30px rgba(0, 0, 0, 0.5);
          font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
          color: #e6edf3;
          pointer-events: auto;
        }
        .cmdp_inline_head {
          display: flex;
          align-items: baseline;
          justify-content: space-between;
          padding: 8px 12px;
          border-bottom: 1px solid rgba(255, 255, 255, 0.08);
        }
        .cmdp_query {
          font-family: 'Cascadia Code', Menlo, monospace;
          color: #9cd2ff;
          font-size: 13px;
        }
        .cmdp_hint_kbd {
          color: #7a828b;
          font-size: 11px;
        }
        .cmdp_list {
          max-height: 260px;
          overflow: auto;
        }
        .cmdp_item {
          display: flex;
          align-items: center;
          gap: 12px;
          padding: 8px 12px;
          cursor: pointer;
          font-size: 13px;
        }
        .cmdp_item.selected {
          background: rgba(120, 160, 255, 0.18);
        }
        .cmdp_label {
          font-family: 'Cascadia Code', Menlo, monospace;
          color: #fff;
        }
        .cmdp_hint {
          color: #8b97a5;
          font-size: 11px;
          margin-left: auto;
        }
        .cmdp_empty {
          padding: 10px 12px;
          color: #8b97a5;
          font-size: 12px;
        }
      `}</style>
    </div>
  );
};

export default CommandPalette;
