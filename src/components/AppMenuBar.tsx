'use client'
import {useEffect, useRef, useState, type KeyboardEvent} from 'react'
import {Check} from 'lucide-react'
export interface MenuAction {label: string; run: () => void; disabled?: boolean; shortcut?: string; checked?: boolean}
export interface AppMenu {label: string; actions: (MenuAction | null)[]}
export default function AppMenuBar({menus, context}: {menus: AppMenu[]; context: string}) {
  const root = useRef<HTMLElement>(null)
  const triggers = useRef<(HTMLButtonElement | null)[]>([])
  const [open, setOpen] = useState<number | null>(null)
  const [active, setActive] = useState(0)
  useEffect(() => {
    const outside = (event: PointerEvent) => {if (!root.current?.contains(event.target as Node)) setOpen(null)}
    document.addEventListener('pointerdown', outside)
    return () => document.removeEventListener('pointerdown', outside)
  }, [])
  function focusItem(last = false) {
    requestAnimationFrame(() => {const items = root.current?.querySelectorAll<HTMLButtonElement>('[role="menu"] button:not(:disabled)'); items?.[last ? items.length - 1 : 0]?.focus()})
  }
  function moveMenu(index: number, expand: boolean) {setActive(index); setOpen(expand ? index : null); triggers.current[index]?.focus(); if (expand) focusItem()}
  function onTrigger(event: KeyboardEvent, index: number) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {event.preventDefault(); setOpen(index); focusItem(event.key === 'ArrowUp')}
    else if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {event.preventDefault(); moveMenu((index + (event.key === 'ArrowRight' ? 1 : menus.length - 1)) % menus.length, open !== null)}
    else if (event.key === 'Escape') {event.preventDefault(); setOpen(null)}
    else if (event.key === 'Home' || event.key === 'End') {event.preventDefault(); moveMenu(event.key === 'Home' ? 0 : menus.length - 1, open !== null)}
  }
  function onMenu(event: KeyboardEvent, index: number) {
    if (event.key === 'Escape') {event.preventDefault(); event.stopPropagation(); setOpen(null); triggers.current[index]?.focus()}
    else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {event.preventDefault(); moveMenu((index + (event.key === 'ArrowRight' ? 1 : menus.length - 1)) % menus.length, true)}
    else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault()
      const items = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'))
      const current = items.indexOf(document.activeElement as HTMLButtonElement)
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (current + (event.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length
      items[next]?.focus()
    } else if (event.key === 'Tab') setOpen(null)
    else if (event.key.length === 1 && !event.ctrlKey && !event.metaKey) {
      const items = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'))
      const current = items.indexOf(document.activeElement as HTMLButtonElement)
      const found = [...items.slice(current + 1), ...items.slice(0, current + 1)].find(item => item.dataset.label?.toLowerCase().startsWith(event.key.toLowerCase()))
      if (found) {event.preventDefault(); found.focus()}
    }
  }
  return <nav ref={root} className="app-menubar relative z-30 flex h-[31px] shrink-0 items-center gap-3 border-b border-solid border-border bg-background px-3" aria-label="Application menu" onBlur={event => {if (!event.currentTarget.contains(event.relatedTarget)) setOpen(null)}}>
    <span className="app-name shrink-0 text-[13px] font-semibold tracking-tight">ILens</span>
    <div className="flex h-full flex-1 items-center gap-0.5" role="menubar" aria-label="Main menu">{menus.map((menu, index) => <div key={menu.label} className="app-menu relative [&_>_button]:text-muted-foreground [&_>_button]:bg-transparent [&_>_button]:border-0 [&_>_button]:py-[5px] [&_>_button]:px-2.5 [&_>_button]:[font:inherit] [&_>_button]:text-[12px] [&_>_button]:rounded-[4px] [&_>_button]:cursor-pointer [&_>_button:hover]:bg-accent [&_>_button:hover]:text-foreground [&_>_button[aria-expanded=true]]:bg-accent [&_>_button[aria-expanded=true]]:text-foreground [&_button:focus-visible]:[outline:1px_solid_var(--ring)] [&_button:focus-visible]:[outline-offset:-1px]">
      <button ref={node => {triggers.current[index] = node}} role="menuitem" aria-haspopup="menu" aria-expanded={open === index} aria-controls={open === index ? `app-menu-${index}` : undefined} tabIndex={active === index ? 0 : -1} onFocus={() => setActive(index)} onKeyDown={event => onTrigger(event, index)} onPointerEnter={() => {if (open !== null) {setOpen(index); setActive(index)}}} onClick={() => {setActive(index); setOpen(open === index ? null : index)}}>{menu.label}</button>
      {open === index && <div id={`app-menu-${index}`} className="app-menu-popup absolute left-0 top-[calc(100%_+_3px)] min-w-[245px] p-1 bg-popover text-foreground border border-solid border-border rounded-[6px] shadow-[var(--shadow)] [&_>_button]:flex [&_>_button]:items-center [&_>_button]:w-full [&_>_button]:gap-[7px] [&_>_button]:border-0 [&_>_button]:rounded-[3px] [&_>_button]:py-[7px] [&_>_button]:px-2 [&_>_button]:bg-transparent [&_>_button]:text-inherit [&_>_button]:[font:inherit] [&_>_button]:text-[12px] [&_>_button]:text-left [&_>_button]:cursor-pointer [&_>_button:hover:not(:disabled)]:bg-accent [&_>_button:focus-visible]:bg-accent [&_>_button:disabled]:text-muted-foreground [&_>_button:disabled]:opacity-45 [&_>_button:disabled]:cursor-default [&_kbd]:ml-auto [&_kbd]:pl-4 [&_kbd]:text-muted-foreground [&_kbd]:text-[10px] [&_kbd]:whitespace-nowrap [&_>_[role=separator]]:border-t [&_>_[role=separator]]:border-solid [&_>_[role=separator]]:border-t-border [&_>_[role=separator]]:my-1 [&_>_[role=separator]]:mx-[-4px]" role="menu" aria-label={menu.label} onKeyDown={event => onMenu(event, index)}>{menu.actions.map((action, i) => action ? <button key={action.label} role={action.checked === undefined ? 'menuitem' : 'menuitemcheckbox'} aria-label={action.label} aria-checked={action.checked} disabled={action.disabled} tabIndex={-1} data-label={action.label} onClick={() => {setOpen(null); triggers.current[index]?.focus(); action.run()}}><span className="menu-check w-3.5 h-3.5 inline-flex shrink-0">{action.checked && <Check size={13} />}</span><span>{action.label}</span>{action.shortcut && <kbd>{action.shortcut}</kbd>}</button> : <div key={`separator-${i}`} role="separator" />)}</div>}
    </div>)}</div><span className="menu-context text-muted-foreground font-mono text-[10px] font-normal leading-normal overflow-hidden text-ellipsis whitespace-nowrap" title={context}>{context}</span>
  </nav>
}
