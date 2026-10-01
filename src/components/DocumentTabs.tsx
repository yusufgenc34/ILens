'use client'
import {ArrowLeft, ArrowRight, Bookmark, X} from 'lucide-react'
import {cn, ui} from '../lib/ui'
import {locationKey, type Location, type Navigation} from '../lib/navigation'
export default function DocumentTabs({state, current, label, onSelect, onClose, onTravel, onBookmark}: {state: Navigation; current: Location | null; label: (location: Location) => string; onSelect: (location: Location) => void; onClose: (location: Location) => void; onTravel: (direction: -1 | 1) => void; onBookmark: () => void}) {
  const key = current && locationKey(current)
  return <div className="document-tab flex h-10 min-h-10 items-center border-b border-border bg-surface">
    <div className="flex shrink-0 items-center gap-0.5 px-1">
      <button className={ui.iconButton} aria-label="Navigate back" title="Back (Alt+Left)" disabled={state.index <= 0} onClick={() => onTravel(-1)}><ArrowLeft size={14}/></button>
      <button className={ui.iconButton} aria-label="Navigate forward" title="Forward (Alt+Right)" disabled={state.index >= state.history.length - 1} onClick={() => onTravel(1)}><ArrowRight size={14}/></button>
    </div>
    <div className="flex h-full min-w-0 flex-1 overflow-x-auto [scrollbar-width:thin]" role="tablist" aria-label="Open documents">
      {state.tabs.map(tab => <div key={locationKey(tab)} className={cn('flex shrink-0 items-center border-r border-border text-xs', key === locationKey(tab) ? 'bg-editor text-foreground' : 'text-muted-foreground')}>
        <button className="max-w-56 truncate px-3 py-2 hover:bg-accent" role="tab" aria-selected={key === locationKey(tab)} title={label(tab)} onClick={() => onSelect(tab)}>{label(tab)}</button>
        <button className="mr-1 rounded p-1 hover:bg-accent" aria-label={`Close ${label(tab)}`} onClick={() => onClose(tab)}><X size={12}/></button>
      </div>)}
    </div>
    <button className={cn(ui.iconButton, 'mx-1 shrink-0')} aria-label="Toggle bookmark" title="Bookmark this declaration" disabled={!current} aria-pressed={!!key && state.bookmarks.some(t => locationKey(t) === key)} onClick={onBookmark}><Bookmark size={14}/></button>
  </div>
}
