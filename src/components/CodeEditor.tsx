'use client'
import {useEffect, useRef} from 'react'
import {EditorState} from '@codemirror/state'
import {EditorView, lineNumbers, highlightActiveLineGutter, keymap, drawSelection, highlightActiveLine} from '@codemirror/view'
import {StreamLanguage, syntaxHighlighting, HighlightStyle, bracketMatching, indentUnit} from '@codemirror/language'
import {csharp} from '@codemirror/legacy-modes/mode/clike'
import {defaultKeymap, history, historyKeymap, indentWithTab} from '@codemirror/commands'
import {tags} from '@lezer/highlight'
import {editorPalette, type EditorSettings} from '../lib/editor-settings'

const csharpLanguage = StreamLanguage.define(csharp)
/** An editable C# editor, styled like the code viewer. `firstLine` numbers its lines as
 * they appear in the compiled source, so diagnostics line up. */
export default function CodeEditor({initial, onChange, settings, theme, label, firstLine = 1}: {initial: string; onChange: (text: string) => void; settings: EditorSettings; theme: string; label: string; firstLine?: number}) {
  const host = useRef<HTMLDivElement>(null)
  const change = useRef(onChange); change.current = onChange
  useEffect(() => {
    if (!host.current) return
    const p = editorPalette(settings.theme, theme)
    const highlighting = HighlightStyle.define([
      {tag: tags.keyword, color: p.keyword, fontWeight: '600'},
      {tag: tags.string, color: p.string},
      {tag: [tags.number, tags.bool, tags.null], color: p.number},
      {tag: tags.comment, color: p.comment, fontStyle: 'italic'},
      {tag: [tags.typeName, tags.className], color: p.type, fontWeight: '500'},
    ])
    const view = new EditorView({parent: host.current, state: EditorState.create({doc: initial, extensions: [
      history(), drawSelection(), bracketMatching(), indentUnit.of('    '), highlightActiveLine(),
      lineNumbers({formatNumber: n => String(n + firstLine - 1)}), highlightActiveLineGutter(),
      keymap.of([indentWithTab, ...historyKeymap, ...defaultKeymap]),
      csharpLanguage, ...(settings.highlighting ? [syntaxHighlighting(highlighting)] : []),
      EditorView.updateListener.of(update => {if (update.docChanged) change.current(update.state.doc.toString())}),
      EditorView.theme({
        '&': {height: '100%', fontSize: `${settings.fontSize}px`, color: p.foreground, backgroundColor: p.background, colorScheme: p.dark ? 'dark' : 'light'},
        '.cm-scroller': {fontFamily: 'var(--mono)', lineHeight: '1.7'},
        '.cm-content': {padding: '12px 0', caretColor: p.foreground},
        '.cm-gutters': {backgroundColor: p.background, color: p.gutter, border: 'none', paddingRight: '8px'},
        '.cm-activeLineGutter, .cm-activeLine': {backgroundColor: p.activeLine},
        '.cm-selectionBackground, &.cm-focused .cm-selectionBackground, .cm-content ::selection': {backgroundColor: p.selection},
        '.cm-cursor': {borderLeftColor: p.foreground},
      }, {dark: p.dark}),
      EditorView.contentAttributes.of({'aria-label': label}),
    ]})})
    return () => view.destroy()
    // The document is owned by the editor after creation.
  }, [settings, theme, label, firstLine]) // eslint-disable-line react-hooks/exhaustive-deps
  return <div ref={host} className="h-full overflow-hidden" />
}
