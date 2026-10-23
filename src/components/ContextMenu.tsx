'use client'
import {useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent} from 'react'
import type {MenuAction} from './AppMenuBar'
// A pointer-positioned menu with the same item model as the menu bar.
export default function ContextMenu({x, y, label, actions, onClose}: {x: number; y: number; label: string; actions: (MenuAction | null)[]; onClose: () => void}) {
  const menu = useRef<HTMLDivElement>(null)
  const [position, setPosition] = useState({left: x, top: y})
  useLayoutEffect(() => {
    const bounds = menu.current?.getBoundingClientRect()
    if (!bounds) return
    setPosition({left: Math.max(4, Math.min(x, window.innerWidth - bounds.width - 4)), top: Math.max(4, Math.min(y, window.innerHeight - bounds.height - 4))})
    menu.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
  }, [x, y])
  useEffect(() => {
    const outside = (event: PointerEvent) => {if (!menu.current?.contains(event.target as Node)) onClose()}
    const dismiss = () => onClose()
    document.addEventListener('pointerdown', outside); window.addEventListener('blur', dismiss); window.addEventListener('resize', dismiss)
    return () => {document.removeEventListener('pointerdown', outside); window.removeEventListener('blur', dismiss); window.removeEventListener('resize', dismiss)}
  }, [onClose])
  function keyboard(event: KeyboardEvent) {
    const items = Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])
    const current = items.indexOf(document.activeElement as HTMLButtonElement)
    // Escape closes only this menu, not drawers or dialogs underneath it.
    if (event.key === 'Escape' || event.key === 'Tab') {event.preventDefault(); event.stopPropagation(); onClose()}
    else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault()
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (current + (event.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length
      items[next]?.focus()
    }
  }
  return <div ref={menu} role="menu" aria-label={label} className="context-menu fixed z-90 min-w-[230px] p-1 bg-popover text-foreground border border-solid border-border rounded-[6px] shadow-[var(--shadow)] text-[12px]" style={position} onKeyDown={keyboard} onContextMenu={event => event.preventDefault()}>
    {actions.map((action, index) => action === null
      ? <div key={`separator-${index}`} role="separator" className="my-1 h-px bg-border" />
      : <button key={action.label} role={action.checked === undefined ? 'menuitem' : 'menuitemcheckbox'} aria-checked={action.checked} disabled={action.disabled} className="flex w-full items-center gap-[7px] rounded-[3px] border-0 bg-transparent px-2 py-[6px] text-left text-inherit [font:inherit] cursor-pointer hover:enabled:bg-accent focus-visible:bg-accent focus-visible:outline-none disabled:text-muted-foreground disabled:cursor-default" onClick={() => {onClose(); action.run()}}>
        <span className="flex-1">{action.label}</span>{action.shortcut && <span className="text-[10px] text-muted-foreground">{action.shortcut}</span>}
      </button>)}
  </div>
}
