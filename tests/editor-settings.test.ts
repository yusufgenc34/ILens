import {expect, it} from 'vitest'
import {defaultEditorSettings, editorPalette, parseEditorSettings} from '../src/lib/editor-settings'

it('restores valid preferences and rejects malformed or injected values', () => {
  const settings = {theme: 'dracula', highlighting: false, fontSize: 16, lineNumbers: false, wrap: true}
  expect(parseEditorSettings(JSON.stringify(settings))).toEqual(settings)
  for (const raw of [null, '', '{', 'null', '[]', '42', '"dark"']) expect(parseEditorSettings(raw)).toEqual(defaultEditorSettings)
  expect(parseEditorSettings(JSON.stringify({theme: 'url(https://example.com)', fontSize: 999999, lineNumbers: 'false', highlighting: 0, wrap: {}}))).toEqual(defaultEditorSettings)
})
it('keeps explicit code palettes independent of the workspace appearance', () => {
  expect(editorPalette('dracula', 'dark')).toEqual(editorPalette('dracula', 'light'))
  expect(editorPalette('light', 'dark').dark).toBe(false)
  expect(editorPalette('neutral', 'light').dark).toBe(false)
  expect(editorPalette('neutral', 'dark').dark).toBe(true)
})
