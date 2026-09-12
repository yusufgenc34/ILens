export const editorThemes = [
  {id: 'neutral', name: 'Neutral · follow appearance'},
  {id: 'dark', name: 'Dark Modern'},
  {id: 'light', name: 'Light Modern'},
  {id: 'dracula', name: 'Dracula'},
  {id: 'monokai', name: 'Monokai'},
  {id: 'solarized', name: 'Solarized Dark'},
] as const
export type EditorTheme = typeof editorThemes[number]['id']
export interface EditorSettings {
  theme: EditorTheme
  highlighting: boolean
  fontSize: number
  lineNumbers: boolean
  wrap: boolean
}
export const editorSettingsKey = 'ilens:editor-settings'
export const fontSizes = [12, 13, 14, 16, 18] as const
export const defaultEditorSettings: EditorSettings = {theme: 'neutral', highlighting: true, fontSize: 13, lineNumbers: true, wrap: false}

// Browser storage is optional and untrusted. Never forward arbitrary CSS into
// the editor from saved preferences.
export function parseEditorSettings(raw: string | null): EditorSettings {
  try {
    const value: unknown = JSON.parse(raw ?? 'null')
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {...defaultEditorSettings}
    const saved = value as Record<string, unknown>
    return {
      theme: editorThemes.some(t => t.id === saved.theme) ? saved.theme as EditorTheme : 'neutral',
      highlighting: typeof saved.highlighting === 'boolean' ? saved.highlighting : true,
      fontSize: fontSizes.some(size => size === saved.fontSize) ? saved.fontSize as number : 13,
      lineNumbers: typeof saved.lineNumbers === 'boolean' ? saved.lineNumbers : true,
      wrap: typeof saved.wrap === 'boolean' ? saved.wrap : false,
    }
  } catch {return {...defaultEditorSettings}}
}
export interface EditorPalette {
  dark: boolean
  background: string
  foreground: string
  gutter: string
  activeLine: string
  selection: string
  keyword: string
  string: string
  number: string
  comment: string
  type: string
  function: string
}
const palettes: Record<Exclude<EditorTheme, 'neutral'>, EditorPalette> = {
  dark: {dark: true, background: '#1e1e1e', foreground: '#d4d4d4', gutter: '#858585', activeLine: '#282828', selection: '#264f78', keyword: '#569cd6', string: '#ce9178', number: '#b5cea8', comment: '#8bad73', type: '#4ec9b0', function: '#dcdcaa'},
  light: {dark: false, background: '#ffffff', foreground: '#24292f', gutter: '#6e7781', activeLine: '#f6f8fa', selection: '#b6d7ff', keyword: '#cf222e', string: '#0a3069', number: '#0550ae', comment: '#57606a', type: '#953800', function: '#8250df'},
  dracula: {dark: true, background: '#282a36', foreground: '#f8f8f2', gutter: '#a0a4ba', activeLine: '#343746', selection: '#44475a', keyword: '#ff79c6', string: '#f1fa8c', number: '#bd93f9', comment: '#a0a4ba', type: '#8be9fd', function: '#50fa7b'},
  monokai: {dark: true, background: '#272822', foreground: '#f8f8f2', gutter: '#a6a69c', activeLine: '#34352e', selection: '#49483e', keyword: '#f92672', string: '#e6db74', number: '#ae81ff', comment: '#a6a69c', type: '#66d9ef', function: '#a6e22e'},
  solarized: {dark: true, background: '#002b36', foreground: '#93a1a1', gutter: '#839496', activeLine: '#073642', selection: '#164b57', keyword: '#b58900', string: '#2aa198', number: '#d33682', comment: '#839496', type: '#268bd2', function: '#859900'},
}
export function editorPalette(theme: EditorTheme, appearance: string): EditorPalette {
  if (theme !== 'neutral') return palettes[theme]
  return {dark: appearance === 'dark', background: 'var(--editor)', foreground: 'var(--foreground)', gutter: 'var(--muted-foreground)', activeLine: 'var(--active-line)', selection: 'var(--selection)', keyword: 'var(--foreground)', string: 'var(--syntax-string)', number: 'var(--foreground)', comment: 'var(--muted-foreground)', type: 'var(--foreground)', function: 'var(--foreground)'}
}
