'use client'
import type { PointerEvent } from 'react'
export default function PaneHandle({name, width, onChange, reverse = false}: {name: string; width: number; onChange: (width: number) => void; reverse?: boolean}) {
  function drag(event: PointerEvent<HTMLDivElement>) {
    event.preventDefault()
    const start = event.clientX
    const target = event.currentTarget
    target.setPointerCapture(event.pointerId)
    const move = (e: globalThis.PointerEvent) => onChange(Math.min(480, Math.max(190, width + (e.clientX - start) * (reverse ? -1 : 1))))
    const stop = () => { target.removeEventListener('pointermove', move); target.removeEventListener('pointerup', stop); target.removeEventListener('pointercancel', stop) }
    target.addEventListener('pointermove', move); target.addEventListener('pointerup', stop); target.addEventListener('pointercancel', stop)
  }
  return <div className="pane-handle relative bg-border cursor-col-resize z-5 touch-none [&:after]:[content:''] [&:after]:absolute [&:after]:[inset:0_-3px] hover:bg-foreground focus-visible:bg-foreground" role="separator" aria-label={`Resize ${name}`} aria-orientation="vertical" aria-valuenow={width} aria-valuemin={190} aria-valuemax={480} tabIndex={0} onPointerDown={drag} onKeyDown={e => {if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {e.preventDefault(); onChange(Math.min(480, Math.max(190, width + (e.key === 'ArrowRight' ? 10 : -10) * (reverse ? -1 : 1))))}}} />
}
