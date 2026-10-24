'use client'
import {cn, ui} from '../lib/ui'
import {useEffect, useLayoutEffect, useState, type CSSProperties} from 'react'
import {ArrowLeft, ArrowRight, X} from 'lucide-react'
export interface TourStep {
  /** CSS selector of the highlighted element; without one (or when it is hidden) the card is centered. */
  target?: string
  title: string
  body: string
}
export const tourSeenKey = 'ilens:tour-seen'
export const tourSteps: TourStep[] = [
  {title: 'Welcome to ILens', body: 'A quick tour of the workspace, using the example assembly. It takes about a minute; press Esc to leave at any time.'},
  {target: '.explorer-pane', title: 'Assembly Explorer', body: 'Assemblies, namespaces, types and members, as in dnSpy. Right-click an item for its actions: Edit Method (C#), Edit IL, Add Method, Analyze and more.'},
  {target: '[role=tablist][aria-label="Code view"]', title: 'Views', body: 'Switch the selected item between reconstructed C#, original IL, its metadata rows and the raw bytes.'},
  {target: '.viewer-container', title: 'Code', body: 'Ctrl/⌘-click a name to go to its definition. Ctrl/⌘ F searches the code, and each opened item gets its own tab above.'},
  {target: '.inspector-pane', title: 'Inspector', body: 'Token, target framework, protection scan and the references of the selected item. Missing dependencies can be added here.'},
  {target: '[aria-label="Main toolbar"]', title: 'Toolbar', body: 'Back and forward, open, save the modified module, search everything (Ctrl/⌘ K), and edit the selected method in IL or C#.'},
  {target: '[aria-label="Application menu"]', title: 'Menus', body: 'File has Save Module and Export to Project, View and Window open the hex editor, metadata tables, search and analyzer. Help restarts this tour.'},
  {title: 'Ready', body: 'Open your own DLL or EXE from File › Open, or drop it anywhere on the page. Files stay on your device and are never executed.'},
]
type Box = {top: number; left: number; width: number; height: number}
/** A step-by-step tour that highlights parts of the workspace. */
export default function Tour({steps, onClose}: {steps: TourStep[]; onClose: () => void}) {
  const [index, setIndex] = useState(0)
  const [box, setBox] = useState<Box | null>(null)
  const [viewport, setViewport] = useState({width: 0, height: 0})
  const step = steps[index]
  useLayoutEffect(() => {
    const measure = () => {
      setViewport({width: window.innerWidth, height: window.innerHeight})
      const element = step.target ? document.querySelector(step.target) : null
      const rect = element?.getBoundingClientRect()
      setBox(rect && rect.width > 0 && rect.height > 0 ? {top: rect.top, left: rect.left, width: rect.width, height: rect.height} : null)
    }
    measure()
    // Layout settles after the example loads and panes resize.
    const timer = setInterval(measure, 250)
    window.addEventListener('resize', measure)
    return () => {clearInterval(timer); window.removeEventListener('resize', measure)}
  }, [step])
  const last = index === steps.length - 1
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
      else if (event.key === 'ArrowRight') setIndex(i => Math.min(steps.length - 1, i + 1))
      else if (event.key === 'ArrowLeft') setIndex(i => Math.max(0, i - 1))
    }
    window.addEventListener('keydown', key)
    return () => window.removeEventListener('keydown', key)
  }, [steps.length, onClose])
  // The card goes beside the highlighted area where there is room, otherwise centered.
  const card = 340, gap = 14, narrow = viewport.width < 640
  let position: CSSProperties = {left: '50%', top: '50%', transform: 'translate(-50%, -50%)'}
  if (box && narrow) position = {left: 12, right: 12, bottom: 16}
  // A large area (the code view) keeps the card inside its bottom-right corner.
  else if (box && box.width * box.height > viewport.width * viewport.height * 0.35) position = {left: box.left + box.width - card - 20, top: box.top + box.height - 250}
  else if (box) {
    const clampTop = (top: number) => Math.max(12, Math.min(top, viewport.height - 240))
    if (box.left + box.width + gap + card < viewport.width) position = {left: box.left + box.width + gap, top: clampTop(box.top + 8)}
    else if (box.left - gap - card > 0) position = {left: box.left - gap - card, top: clampTop(box.top + 8)}
    else position = {left: Math.max(12, Math.min(box.left, viewport.width - card - 12)), top: box.top + box.height + gap < viewport.height - 200 ? box.top + box.height + gap : clampTop(box.top - 200)}
  }
  return <div className="tour fixed inset-0 z-90" role="dialog" aria-modal="true" aria-labelledby="tour-title">
    {box ? <div className="pointer-events-none fixed rounded-[8px] border-2 border-solid border-[var(--primary)] shadow-[0_0_0_9999px_#000a] transition-all duration-200 motion-reduce:transition-none" style={{top: box.top - 4, left: box.left - 4, width: box.width + 8, height: box.height + 8}} /> : <div className="fixed inset-0 bg-[#000a]" />}
    <div className="fixed w-[340px] max-w-[calc(100vw-24px)] rounded-[10px] border border-solid border-border bg-popover p-5 text-foreground shadow-[var(--shadow)]" style={position}>
      <div className="mb-2 flex items-center justify-between text-[11px] text-muted-foreground"><span>Step {index + 1} of {steps.length}</span><button className={cn(ui.iconButton)} aria-label="Close tour" onClick={onClose}><X size={15} /></button></div>
      <h2 id="tour-title" className="m-0 mb-2 text-[16px] font-semibold">{step.title}</h2>
      <p className="m-0 text-[13px] leading-[1.6] text-muted-foreground">{step.body}</p>
      <div className="mt-4 flex items-center gap-1" aria-hidden="true">{steps.map((_, i) => <span key={i} className={cn('h-1.5 rounded-full transition-all', i === index ? 'w-4 bg-[var(--primary)]' : 'w-1.5 bg-[var(--input)]')} />)}</div>
      <div className="mt-4 flex items-center gap-2">
        {!last && <button className={cn(ui.button, ui.subtle, 'mr-auto px-2 text-[12px]')} onClick={onClose}>Skip</button>}
        {index > 0 && <button className={cn(ui.button, ui.subtle, last && 'mr-auto')} onClick={() => setIndex(index - 1)}><ArrowLeft size={14} />Back</button>}
        <button className={cn(ui.button, ui.primary, last && index === 0 && 'ml-auto')} autoFocus onClick={() => last ? onClose() : setIndex(index + 1)}>{last ? 'Finish' : 'Next'}{!last && <ArrowRight size={14} />}</button>
      </div>
    </div>
  </div>
}
