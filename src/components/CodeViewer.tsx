'use client'
import {useEffect, useMemo, useRef} from 'react'
import {Compartment, EditorState} from '@codemirror/state'
import {EditorView, lineNumbers, highlightActiveLineGutter, keymap, drawSelection, highlightActiveLine} from '@codemirror/view'
import {StreamLanguage, syntaxHighlighting, HighlightStyle, bracketMatching} from '@codemirror/language'
import {csharp} from '@codemirror/legacy-modes/mode/clike'
import {search, searchKeymap, highlightSelectionMatches} from '@codemirror/search'
import {defaultKeymap} from '@codemirror/commands'
import {tags} from '@lezer/highlight'
import {editorPalette, type EditorSettings} from '../lib/editor-settings'

const csharpLanguage = StreamLanguage.define(csharp)
const il = StreamLanguage.define({token(stream) {if (stream.match('//')) {stream.skipToEnd(); return 'comment'} if (stream.match(/IL_[0-9a-fA-F]+/)) return 'labelName'; if (stream.match(/0x[0-9a-fA-F]+/)) return 'number'; if (stream.match(/"(?:[^"\\]|\\.)*"/)) return 'string'; if (stream.match(/[a-z][a-z0-9_.]+/)) return 'keyword'; stream.next(); return null}})

export default function CodeViewer({text, language, theme, settings, onNavigate, label}: {text: string; language: string; theme: string; settings: EditorSettings; onNavigate?: (token: number) => void; label?: string}) {
  const host = useRef<HTMLDivElement>(null)
  const editor = useRef<EditorView | null>(null)
  const options = useMemo(() => new Compartment(), [])
  const navigate = useRef(onNavigate)
  navigate.current = onNavigate
  // Reconfigure the live editor to preserve selection and scroll on theme changes.
  useEffect(() => {
    if (!host.current) return
    const view = new EditorView({parent: host.current, state: EditorState.create({extensions: [
      EditorState.readOnly.of(true), EditorView.editable.of(false), drawSelection(), bracketMatching(),
      keymap.of([...searchKeymap, ...defaultKeymap]), search({top: true}), highlightSelectionMatches(),
      options.of([]),
      EditorView.domEventHandlers({click(event, view) {
        if (!navigate.current || (!event.ctrlKey && !event.metaKey)) return false
        const position = view.posAtCoords({x: event.clientX, y: event.clientY})
        if (position === null) return false
        const line = view.state.doc.lineAt(position)
        const match = [...line.text.matchAll(/0x[0-9a-fA-F]{8}/g)].find(m => position - line.from >= m.index! && position - line.from <= m.index! + m[0].length)
        if (!match) return false
        navigate.current(parseInt(match[0], 16)); return true
      }}),
    ]})})
    editor.current = view
    return () => {view.destroy(); editor.current = null}
  }, [options])
  useEffect(() => {
    const view = editor.current
    if (view && view.state.doc.toString() !== text) view.dispatch({changes: {from: 0, to: view.state.doc.length, insert: text}, selection: {anchor: 0}, effects: EditorView.scrollIntoView(0)})
  }, [text])
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
  }, [options, language, theme, settings, label])
  return <div ref={host} className="code-host h-full overflow-hidden" data-editor-theme={settings.theme} data-highlighting={settings.highlighting} />
}
