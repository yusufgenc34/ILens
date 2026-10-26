'use client'
import {useEffect, useMemo, useRef} from 'react'
import {Compartment, EditorState} from '@codemirror/state'
import {EditorView, lineNumbers, highlightActiveLineGutter, keymap, drawSelection, highlightActiveLine, Decoration, hoverTooltip} from '@codemirror/view'
import {StreamLanguage, syntaxHighlighting, HighlightStyle, bracketMatching} from '@codemirror/language'
import {csharp} from '@codemirror/legacy-modes/mode/clike'
import {search, searchKeymap, highlightSelectionMatches} from '@codemirror/search'
import {defaultKeymap} from '@codemirror/commands'
import {tags} from '@lezer/highlight'
import type {SourceReference} from '../lib/types'
import {editorPalette, type EditorSettings} from '../lib/editor-settings'

const csharpLanguage = StreamLanguage.define(csharp)
const il = StreamLanguage.define({token(stream) {if (stream.match('//')) {stream.skipToEnd(); return 'comment'} if (stream.match(/IL_[0-9a-fA-F]+/)) return 'labelName'; if (stream.match(/0x[0-9a-fA-F]+/)) return 'number'; if (stream.match(/"(?:[^"\\]|\\.)*"/)) return 'string'; if (stream.match(/[a-z][a-z0-9_.]+/)) return 'keyword'; stream.next(); return null}})

export default function CodeViewer({text, language, theme, settings, onNavigate, onReferenceNavigate, references, documentKey, label}: {references?: SourceReference[]; documentKey?: string; onReferenceNavigate?: (token: number) => void; text: string; language: string; theme: string; settings: EditorSettings; onNavigate?: (token: number) => void; label?: string}) {
  const host = useRef<HTMLDivElement>(null)
  const editor = useRef<EditorView | null>(null)
  const options = useMemo(() => new Compartment(), [])
  const navigate = useRef(onNavigate)
  navigate.current = onNavigate
  const referenceNavigate = useRef(onReferenceNavigate); referenceNavigate.current = onReferenceNavigate
  const sourceReferences = useRef(references); sourceReferences.current = references
  const activeDocument = useRef<string | undefined>(undefined)
  const savedPositions = useRef(new Map<string, {anchor:number; head:number; top:number; left:number}>())
  const live = useRef<{key: string; top: number; left: number} | null>(null)
  const hidden = useRef(false)
  const referenceAt = (position: number) => sourceReferences.current?.find(r => position >= r.start && position < r.end)
  // Reconfigure the live editor to preserve selection and scroll on theme changes.
  useEffect(() => {
    if (!host.current) return
    const view = new EditorView({parent: host.current, state: EditorState.create({extensions: [
      EditorState.readOnly.of(true), EditorView.editable.of(false), drawSelection(), bracketMatching(),
      keymap.of([{key:'F12', run:view => {const reference = referenceAt(view.state.selection.main.head); if (!reference || !referenceNavigate.current) return false; referenceNavigate.current(reference.token); return true}}, ...searchKeymap, ...defaultKeymap]),
      hoverTooltip((_view, position) => {const reference = referenceAt(position); if (!reference) return null; return {pos:reference.start, end:reference.end, create:() => {const dom = document.createElement('div'); dom.className = 'px-2 py-1 text-xs'; dom.textContent = `Go to definition · 0x${reference.token.toString(16).toUpperCase().padStart(8,'0')} · click or F12`; return {dom}}}}), search({top: true}), highlightSelectionMatches(),
      options.of([]),
      EditorView.domEventHandlers({click(event, view) {
        const modified = event.ctrlKey || event.metaKey
        const position = view.posAtCoords({x: event.clientX, y: event.clientY})
        if (position === null) return false
        // References open on a plain click, as in dnSpy; dragging to select text or
        // extending a selection with Shift never navigates.
        const reference = referenceAt(position)
        if (reference && referenceNavigate.current && !event.shiftKey && !event.altKey && (modified || view.state.selection.main.empty)) {referenceNavigate.current(reference.token); return true}
        if (!navigate.current || !modified) return false
        const line = view.state.doc.lineAt(position)
        const match = [...line.text.matchAll(/0x[0-9a-fA-F]{8}/g)].find(m => position - line.from >= m.index! && position - line.from <= m.index! + m[0].length)
        if (!match) return false
        navigate.current(parseInt(match[0], 16)); return true
      }}),
    ]})})
    editor.current = view
    // The last scroll position of the shown document. Reading it later is unreliable:
    // a hidden (display: none) editor reports 0 in Firefox.
    const track = () => {if (activeDocument.current && !hidden.current) live.current = {key: activeDocument.current, top: view.scrollDOM.scrollTop, left: view.scrollDOM.scrollLeft}}
    view.scrollDOM.addEventListener('scroll', track, {passive: true})
    return () => {view.scrollDOM.removeEventListener('scroll', track); view.destroy(); editor.current = null}
  }, [options])
  useEffect(() => {
    const view = editor.current
    if (!view) return
    // Another view (PE, resources) covers the editor; its document stays loaded.
    if (activeDocument.current && !documentKey) {hidden.current = true; return}
    const position = () => {const last = live.current; return last && last.key === activeDocument.current ? last : {top: view.scrollDOM.scrollTop, left: view.scrollDOM.scrollLeft}}
    if (view.state.doc.toString() === text && activeDocument.current === documentKey) {
      if (!hidden.current) return
      // Shown again: put the document back where it was.
      hidden.current = false
      const {top, left} = position()
      const frame = requestAnimationFrame(() => {view.scrollDOM.scrollTop = top; view.scrollDOM.scrollLeft = left})
      return () => cancelAnimationFrame(frame)
    }
    hidden.current = false
    if (activeDocument.current) {
      const selection = view.state.selection.main
      const {top, left} = position()
      savedPositions.current.set(activeDocument.current, {anchor:selection.anchor, head:selection.head, top, left})
      if (savedPositions.current.size > 128) savedPositions.current.delete(savedPositions.current.keys().next().value!)
    }
    const saved = documentKey && savedPositions.current.get(documentKey)
    activeDocument.current = documentKey
    view.dispatch({changes:{from:0, to:view.state.doc.length, insert:text}, selection:{anchor:Math.min(saved ? saved.anchor : 0, text.length), head:Math.min(saved ? saved.head : 0, text.length)}})
    const frame = requestAnimationFrame(() => {view.scrollDOM.scrollTop = saved ? saved.top : 0; view.scrollDOM.scrollLeft = saved ? saved.left : 0})
    return () => cancelAnimationFrame(frame)
  }, [text, documentKey])
  useEffect(() => {
    const p = editorPalette(settings.theme, theme)
    const highlighting = HighlightStyle.define([
      {tag: tags.keyword, color: p.keyword, fontWeight: '600'},
      {tag: tags.string, color: p.string},
      {tag: [tags.number, tags.bool, tags.null], color: p.number},
      {tag: tags.comment, color: p.comment, fontStyle: 'italic'},
      {tag: [tags.typeName, tags.className], color: p.type, fontWeight: '500'},
      {tag: tags.function(tags.variableName), color: p.function},
      {tag: tags.labelName, color: p.gutter},
    ])
    editor.current?.dispatch({effects: options.reconfigure([
      language === 'C#' ? csharpLanguage : il,
      ...(settings.highlighting ? [syntaxHighlighting(highlighting)] : []),
      ...(settings.lineNumbers ? [lineNumbers(), highlightActiveLineGutter()] : []),
      EditorView.decorations.of(Decoration.set((references ?? []).filter(r => r.start >= 0 && r.end > r.start && r.end <= (editor.current?.state.doc.length ?? 0)).map(r => Decoration.mark({class:'cursor-pointer underline decoration-dotted decoration-muted-foreground/40 underline-offset-2 hover:decoration-solid hover:decoration-current'}).range(r.start,r.end)), true)),
      highlightActiveLine(), ...(settings.wrap ? [EditorView.lineWrapping] : []),
      EditorView.theme({
        '&': {height: '100%', fontSize: `${settings.fontSize}px`, color: p.foreground, backgroundColor: p.background, colorScheme: p.dark ? 'dark' : 'light'},
        '.cm-scroller': {fontFamily: 'var(--mono)', lineHeight: '1.8'},
        '.cm-content': {padding: '20px 0', caretColor: p.foreground},
        '.cm-line': {padding: '0 24px 0 12px'},
        '.cm-gutters': {backgroundColor: p.background, color: p.gutter, border: 'none', paddingRight: '10px', minWidth: '54px'},
        '.cm-activeLineGutter, .cm-activeLine': {backgroundColor: p.activeLine},
        '.cm-selectionBackground, &.cm-focused .cm-selectionBackground, .cm-content ::selection': {backgroundColor: p.selection},
        '.cm-panels': {backgroundColor: p.background, color: p.foreground},
        '.cm-searchMatch': {backgroundColor: p.selection, outline: `1px solid ${p.gutter}`},
        '.cm-cursor': {borderLeftColor: p.foreground},
      }, {dark: p.dark}),
      EditorView.contentAttributes.of({'aria-label': label ?? `${language} code viewer`, tabindex: '0'}),
    ])})
  }, [options, language, theme, settings, label, references, text, documentKey])
  return <div ref={host} className="code-host h-full overflow-hidden" data-editor-theme={settings.theme} data-highlighting={settings.highlighting} />
}
