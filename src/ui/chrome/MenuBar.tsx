import { useEffect, useRef, useState, type JSX } from 'react';
import { MenuPopup, cmd, sep, type MenuItem } from '../controls/Menu';
import { PANEL_COMMAND_IDS, SPEED_COMMAND_IDS } from '../services/commands';
import { openExample, openPath } from '../services/files';
import { SHOWCASE } from '../services/examples';
import { useSettings } from '../state/settings';
import { useDevices } from '../state/devices';
import { selectDevice } from '../services/device';
import { baseName } from '../services/debugInfo';

interface TopMenu {
  title: string;
  /** Index of the mnemonic letter in title. */
  mnemonic: number;
  items: () => MenuItem[];
}

const MENUS: TopMenu[] = [
  {
    title: 'File',
    mnemonic: 0,
    items: () => [
      cmd('file.newAsm'), cmd('file.newC'), cmd('file.newMc'), cmd('file.open'),
      { kind: 'sub', label: 'Open Example', items: () => SHOWCASE.map((e) => ({ kind: 'action', label: e.title, run: () => openExample(e.name) })) },
      sep, cmd('file.save'), cmd('file.saveAs'), cmd('file.saveAll'), cmd('file.close'),
      sep, cmd('file.import'), cmd('file.exportHex'),
      sep,
      {
        kind: 'sub',
        label: 'Recent Files',
        disabled: useSettings.getState().recentFiles.length === 0,
        items: () => useSettings.getState().recentFiles.map((p, i) => ({ kind: 'action', label: `${i + 1}  ${baseName(p)}`, run: () => void openPath(p) })),
      },
      sep, cmd('file.exit'),
    ],
  },
  {
    title: 'Edit',
    mnemonic: 0,
    items: () => [cmd('edit.undo'), cmd('edit.redo'), sep, cmd('edit.cut'), cmd('edit.copy'), cmd('edit.paste'), cmd('edit.selectAll'), sep, cmd('edit.find'), cmd('edit.replace'), cmd('edit.gotoLine')],
  },
  {
    title: 'View',
    mnemonic: 0,
    items: () => [cmd('view.startPage'), sep, ...PANEL_COMMAND_IDS.map((id) => cmd(id)), sep, cmd('view.resetLayout')],
  },
  { title: 'Build', mnemonic: 0, items: () => [cmd('build.build'), sep, cmd('build.toMachineCode'), sep, cmd('build.options')] },
  {
    title: 'Debug',
    mnemonic: 0,
    items: () => [
      cmd('debug.start'), cmd('debug.pause'), cmd('debug.stop'), cmd('debug.reset'),
      sep, cmd('debug.stepInto'), cmd('debug.stepOver'), cmd('debug.stepOut'), cmd('debug.runToCursor'),
      sep, cmd('debug.toggleBreakpoint'), cmd('debug.clearBreakpoints'),
      sep, cmd('debug.sourceStepping'),
      { kind: 'sub', label: 'Simulation Speed', items: () => SPEED_COMMAND_IDS.map((id) => cmd(id)) },
    ],
  },
  {
    title: 'Device',
    mnemonic: 1,
    items: () => {
      const cur = useSettings.getState().deviceId;
      return [
        ...useDevices.getState().devices.map((d): MenuItem => ({ kind: 'action', label: `${d.name}  (${d.flashSize} B flash, ${d.sramSize} B SRAM)`, checked: d.id === cur, run: () => selectDevice(d.id) })),
        sep, cmd('device.info'), cmd('device.chip'),
        sep, cmd('device.fuses'), cmd('device.supply'),
      ];
    },
  },
  { title: 'Tools', mnemonic: 0, items: () => [cmd('tools.toolchain')] },
  { title: 'Help', mnemonic: 0, items: () => [cmd('help.isa'), cmd('help.include'), cmd('help.toolchain'), sep, cmd('help.updates'), cmd('help.about')] },
];

export function MenuBar(): JSX.Element {
  const [open, setOpen] = useState<number | null>(null);
  const [mnemonics, setMnemonics] = useState(false);
  const refs = useRef<(HTMLDivElement | null)[]>([]);

  useEffect(() => {
    if (open === null) return;
    const close = () => setOpen(null);
    window.addEventListener('mousedown', close);
    window.addEventListener('blur', close);
    return () => {
      window.removeEventListener('mousedown', close);
      window.removeEventListener('blur', close);
    };
  }, [open]);

  // Alt shows mnemonics; Alt+letter opens a menu.
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key === 'Alt') setMnemonics(true);
      if (e.altKey && !e.ctrlKey && !e.metaKey && e.key.length === 1) {
        const i = MENUS.findIndex((m) => m.title[m.mnemonic].toLowerCase() === e.key.toLowerCase());
        if (i >= 0) {
          e.preventDefault();
          setOpen(i);
        }
      }
    };
    const up = (e: KeyboardEvent) => {
      if (e.key === 'Alt') setMnemonics(false);
    };
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
    };
  }, []);

  const rect = open !== null ? refs.current[open]?.getBoundingClientRect() : undefined;
  return (
    <div className={`menubar${mnemonics ? ' show-mnemonics' : ''}`}>
      {MENUS.map((m, i) => (
        <div
          key={m.title}
          ref={(el) => {
            refs.current[i] = el;
          }}
          className={`menubar-item${open === i ? ' open' : ''}`}
          onMouseDown={(e) => {
            e.stopPropagation();
            setOpen(open === i ? null : i);
          }}
          onMouseEnter={() => open !== null && setOpen(i)}
        >
          {m.title.slice(0, m.mnemonic)}
          <u>{m.title[m.mnemonic]}</u>
          {m.title.slice(m.mnemonic + 1)}
        </div>
      ))}
      {open !== null && rect && (
        <MenuPopup
          key={open}
          items={MENUS[open].items()}
          x={rect.left}
          y={rect.bottom}
          onClose={() => setOpen(null)}
          onLeft={() => setOpen((open + MENUS.length - 1) % MENUS.length)}
          onRight={() => setOpen((open + 1) % MENUS.length)}
        />
      )}
    </div>
  );
}
