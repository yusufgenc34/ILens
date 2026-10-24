import {expect, it} from 'vitest'
import {defaultEditorSettings, editorPalette, parseEditorSettings, stripTokenComments} from '../src/lib/editor-settings'

it('restores valid preferences and rejects malformed or injected values', () => {
  const settings = {theme: 'monokai', highlighting: false, fontSize: 16, lineNumbers: false, wrap: true, explorerTokens: false, tokenComments: false, hideGenerated: true}
  expect(parseEditorSettings(JSON.stringify(settings))).toEqual(settings)
  for (const raw of [null, '', '{', 'null', '[]', '42', '"dark"']) expect(parseEditorSettings(raw)).toEqual(defaultEditorSettings)
  expect(parseEditorSettings(JSON.stringify({theme: 'url(https://example.com)', fontSize: 999999, lineNumbers: 'false', highlighting: 0, wrap: {}}))).toEqual(defaultEditorSettings)
})
it('defaults to the Dark Modern code theme', () => expect(defaultEditorSettings.theme).toBe('dark'))
it('treats Dracula saved before version 2 as the former default', () => {
  expect(parseEditorSettings(JSON.stringify({theme: 'dracula'})).theme).toBe('dark')
  expect(parseEditorSettings(JSON.stringify({theme: 'dracula', version: 2})).theme).toBe('dracula')
})
it('keeps explicit code palettes independent of the workspace appearance', () => {
  expect(editorPalette('dracula', 'dark')).toEqual(editorPalette('dracula', 'light'))
  expect(editorPalette('light', 'dark').dark).toBe(false)
  expect(editorPalette('neutral', 'light').dark).toBe(false)
  expect(editorPalette('neutral', 'dark').dark).toBe(true)
})
it('removes token comments and keeps navigation ranges on their text', () => {
  const text = 'class A\n{\n    // Token: 0x06000001 RID: 1 RVA: 0x00002050\n    void M() { N(); }\n}\n'
  const at = text.indexOf('N()')
  const {text: shown, references} = stripTokenComments(text, [{start: at, end: at + 1, token: 7}])
  expect(shown).toBe('class A\n{\n    void M() { N(); }\n}\n')
  expect(shown.slice(references![0].start, references![0].end)).toBe('N')
})
