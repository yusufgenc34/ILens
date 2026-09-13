'use client'
import {cn, ui} from '../lib/ui'
import {useEffect, useRef} from 'react'
import {Settings2, X} from 'lucide-react'
import {defaultEditorSettings, editorThemes, fontSizes, type EditorSettings, type EditorTheme} from '../lib/editor-settings'
import CodeViewer from './CodeViewer'

const preview = `// Syntax preview\npublic static string Describe(int count)\n{\n    if (count > 0)\n        return "Ready to explore";\n    return string.Empty;\n}`

export default function SettingsDialog({settings, onChange, appearance, onAppearance, onClose}: {settings: EditorSettings; onChange: (settings: EditorSettings) => void; appearance: string; onAppearance: (theme: string) => void; onClose: () => void}) {
  const dialog = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const element = dialog.current
    const previousFocus = document.activeElement
    element?.showModal()
    return () => {
      element?.close()
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus({preventScroll: true})
    }
  }, [])
  const update = (patch: Partial<EditorSettings>) => onChange({...settings, ...patch})
  return <dialog ref={dialog} className={cn(ui.dialog)} aria-labelledby="settings-title" aria-describedby="settings-description" onCancel={e => {e.preventDefault(); onClose()}} onClick={e => {
    if (e.target !== e.currentTarget) return
    const box = e.currentTarget.getBoundingClientRect()
    if (e.clientX < box.left || e.clientX > box.right || e.clientY < box.top || e.clientY > box.bottom) onClose()
  }}>
    <header className={cn(ui.dialogHeader)}><div><h2 id="settings-title"><Settings2 size={18} />Settings</h2><p id="settings-description">Make the code viewer your own. Changes apply immediately.</p></div><button className={cn(ui.iconButton)} aria-label="Close settings" onClick={onClose}><X size={18} /></button></header>
    <div className={cn(ui.dialogBody)}>
      <div className="setting-row flex justify-between items-center gap-5 py-3.5 px-0 border-b border-solid border-b-border [&_label]:font-medium [&_small]:block [&_small]:text-[11px] [&_small]:font-normal [&_small]:text-muted-foreground [&_small]:mt-1 [&_select]:min-w-[195px] [&_select]:max-w-[55%] [&_select]:border [&_select]:border-solid [&_select]:border-input [&_select]:bg-background [&_select]:text-foreground [&_select]:py-2 [&_select]:px-2.5 [&_select]:rounded-[6px] [&_select]:text-[12px]"><label htmlFor="appearance"><span id="appearance-label">Appearance</span><small id="appearance-hint">Workspace, menus, and panels</small></label><select id="appearance" aria-labelledby="appearance-label" aria-describedby="appearance-hint" value={appearance} onChange={e => onAppearance(e.target.value)}><option value="dark">Dark</option><option value="light">Light</option></select></div>
      <div className="setting-row flex justify-between items-center gap-5 py-3.5 px-0 border-b border-solid border-b-border [&_label]:font-medium [&_small]:block [&_small]:text-[11px] [&_small]:font-normal [&_small]:text-muted-foreground [&_small]:mt-1 [&_select]:min-w-[195px] [&_select]:max-w-[55%] [&_select]:border [&_select]:border-solid [&_select]:border-input [&_select]:bg-background [&_select]:text-foreground [&_select]:py-2 [&_select]:px-2.5 [&_select]:rounded-[6px] [&_select]:text-[12px]"><label htmlFor="editor-theme"><span id="code-theme-label">Code theme</span><small id="code-theme-hint">Independent of the workspace appearance</small></label><select id="editor-theme" aria-labelledby="code-theme-label" aria-describedby="code-theme-hint" value={settings.theme} onChange={e => update({theme: e.target.value as EditorTheme})}>{editorThemes.map(theme => <option key={theme.id} value={theme.id}>{theme.name}</option>)}</select></div>
      <div className="setting-row flex justify-between items-center gap-5 py-3.5 px-0 border-b border-solid border-b-border [&_label]:font-medium [&_small]:block [&_small]:text-[11px] [&_small]:font-normal [&_small]:text-muted-foreground [&_small]:mt-1 [&_select]:min-w-[195px] [&_select]:max-w-[55%] [&_select]:border [&_select]:border-solid [&_select]:border-input [&_select]:bg-background [&_select]:text-foreground [&_select]:py-2 [&_select]:px-2.5 [&_select]:rounded-[6px] [&_select]:text-[12px]"><label htmlFor="editor-font-size">Font size</label><select id="editor-font-size" value={settings.fontSize} onChange={e => update({fontSize: Number(e.target.value)})}>{fontSizes.map(size => <option key={size} value={size}>{size} px</option>)}</select></div>
      <div className="settings-toggles flex flex-wrap gap-4.5 py-5 px-0 [&_label]:flex [&_label]:items-center [&_label]:gap-[7px] [&_label]:text-[12px] [&_label]:cursor-pointer [&_input]:m-0 [&_input]:w-3.5 [&_input]:h-3.5 [&_input]:accent-primary">
        <label><input type="checkbox" checked={settings.highlighting} onChange={e => update({highlighting: e.target.checked})} />Syntax highlighting</label>
        <label><input type="checkbox" checked={settings.lineNumbers} onChange={e => update({lineNumbers: e.target.checked})} />Line numbers</label>
        <label><input type="checkbox" checked={settings.wrap} onChange={e => update({wrap: e.target.checked})} />Word wrap</label>
      </div>
      <div className="settings-preview-label flex justify-between text-muted-foreground text-[11px] mb-2">Preview<span>C#</span></div>
      <div className="settings-preview h-60 overflow-hidden border border-solid border-border rounded-[6px]"><CodeViewer text={preview} language="C#" label="Syntax theme preview" theme={appearance} settings={settings} /></div>
    </div>
    <footer className={cn(ui.dialogFooter)}><button className={cn(ui.button, ui.subtle)} onClick={() => onChange({...defaultEditorSettings})}>Reset editor settings</button><button className={cn(ui.button, ui.primary)} onClick={onClose}>Done</button></footer>
  </dialog>
}
