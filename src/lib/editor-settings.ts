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
  /** dnSpy's `@0200000A` token suffixes in the Assembly Explorer. */
  explorerTokens: boolean
  /** `// Token: … RID: … RVA: …` comments above members in decompiled code. */
  tokenComments: boolean
  /** Hide compiler-generated types and members (`<>c`, `<Main>d__0`, backing fields). */
  hideGenerated: boolean
}
export const editorSettingsKey = 'ilens:editor-settings'
export const fontSizes = [12, 13, 14, 16, 18] as const
export const defaultEditorSettings: EditorSettings = {theme: 'dark', highlighting: true, fontSize: 13, lineNumbers: true, wrap: false, explorerTokens: true, tokenComments: true, hideGenerated: false}

// Browser storage is optional and untrusted. Never forward arbitrary CSS into
// the editor from saved preferences.
export function parseEditorSettings(raw: string | null): EditorSettings {
  try {
    const value: unknown = JSON.parse(raw ?? 'null')
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {...defaultEditorSettings}
    const saved = value as Record<string, unknown>
    return {
      // Settings saved before version 2 hold Dracula as the former default, not a choice.
      theme: editorThemes.some(t => t.id === saved.theme) && !(saved.version !== 2 && saved.theme === 'dracula') ? saved.theme as EditorTheme : defaultEditorSettings.theme,
      highlighting: typeof saved.highlighting === 'boolean' ? saved.highlighting : true,
      fontSize: fontSizes.some(size => size === saved.fontSize) ? saved.fontSize as number : 13,
      lineNumbers: typeof saved.lineNumbers === 'boolean' ? saved.lineNumbers : true,
      wrap: typeof saved.wrap === 'boolean' ? saved.wrap : false,
      explorerTokens: typeof saved.explorerTokens === 'boolean' ? saved.explorerTokens : true,
      tokenComments: typeof saved.tokenComments === 'boolean' ? saved.tokenComments : true,
      hideGenerated: typeof saved.hideGenerated === 'boolean' ? saved.hideGenerated : false,
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

const tokenComment = /^[ \t]*\/\/ (\([a-z]+\) )?Token: 0x[0-9A-F]{8}\b.*(\r?\n|$)/gm
/** Removes dnSpy-style token comment lines, keeping navigation ranges on their text. */
export function stripTokenComments<R extends {start: number; end: number}>(text: string, references: R[] | undefined): {text: string; references: R[] | undefined} {
  const removed: [number, number][] = []
  for (const match of text.matchAll(tokenComment)) removed.push([match.index!, match.index! + match[0].length])
  if (!removed.length) return {text, references}
  let out = '', last = 0
  for (const [start, end] of removed) {out += text.slice(last, start); last = end}
  out += text.slice(last)
  // UTF-16 offsets shift by the length removed before them.
  const shift = (offset: number) => removed.reduce((total, [start, end]) => total + (end <= offset ? end - start : 0), 0)
  return {text: out, references: references?.filter(r => !removed.some(([start, end]) => r.start >= start && r.end <= end)).map(r => ({...r, start: r.start - shift(r.start), end: r.end - shift(r.start)}))}
}
/** The workspace chrome in the code theme's colors, so panels, menus and dialogs match
 * the code view (the theme's sidebar, borders and accent rather than neutral black).
 * Only dark code themes in the dark appearance; otherwise the neutral tokens apply. */
export function chromeStyle(theme: EditorTheme, appearance: string): Record<string, string> | undefined {
  if (theme === 'neutral' || appearance !== 'dark') return undefined
  const p = palettes[theme]
  if (!p.dark) return undefined
  const mix = (color: string, other: string, amount: number) => `color-mix(in srgb, ${color}, ${other} ${amount}%)`
  return {
    '--editor': p.background,
    '--background': mix(p.background, '#000', 28),
    '--surface': mix(p.background, '#000', 16),
    '--card': mix(p.background, '#fff', 3),
    '--popover': mix(p.background, '#fff', 3),
    '--border': mix(p.background, '#000', 42),
    '--muted': p.activeLine,
    '--accent': p.activeLine,
    '--accent-dim': p.activeLine,
    '--hover': p.activeLine,
    '--secondary': p.activeLine,
    '--input': p.selection,
    '--selection': p.selection,
    '--active-line': p.activeLine,
    '--foreground': p.foreground,
    '--text': p.foreground,
    '--card-foreground': p.foreground,
    '--accent-foreground': p.foreground,
    '--secondary-foreground': p.foreground,
    '--muted-foreground': p.gutter,
    '--faint': mix(p.gutter, p.background, 25),
    '--ring': p.number,
    '--primary': p.number,
    '--primary-hover': mix(p.number, '#fff', 15),
    '--primary-foreground': p.background,
    '--syntax-string': p.string,
    '--tree-type': p.type,
    '--tree-interface': p.type,
    '--tree-value-type': p.type,
    '--tree-delegate': p.type,
    '--tree-method': p.function,
    '--tree-namespace': p.foreground,
    '--tree-assembly': p.foreground,
    '--shadow': '0 12px 32px #0006',
  }
}
