'use client'
import {cn, ui} from '../lib/ui'
import {useEffect, useRef, useState} from 'react'
import {Check, LoaderCircle, Plus, Trash2, X} from 'lucide-react'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic, tokenHex, type Diagnostic, type NewMemberRequest} from '../lib/types'
const types = ['int', 'long', 'bool', 'string', 'object', 'double', 'float', 'byte', 'char', 'int[]', 'string[]']
/** dnSpy's Add Field / Add Method: declares a member; method bodies start as a default return. */
export default function AddMemberDialog({client, assembly, owner, ownerName, kind, onClose, onAdded}: {client: DecompilerClient; assembly: number; owner: number; ownerName: string; kind: 'field' | 'method'; onClose: () => void; onAdded: (token: number) => void}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const [member, setMember] = useState<NewMemberRequest>({kind, name: '', type: kind === 'method' ? 'void' : 'int', access: 'public', static: false, parameters: []})
  const [busy, setBusy] = useState(false); const [error, setError] = useState<Diagnostic | null>(null)
  useEffect(() => {const previous = document.activeElement; dialog.current?.showModal(); return () => {dialog.current?.close(); if (previous instanceof HTMLElement && previous.isConnected) previous.focus()}}, [])
  const update = (patch: Partial<NewMemberRequest>) => {setMember(m => ({...m, ...patch})); setError(null)}
  const setParameter = (index: number, patch: Partial<{name: string; type: string}>) => update({parameters: member.parameters.map((p, i) => i === index ? {...p, ...patch} : p)})
  async function submit() {
    setBusy(true); setError(null)
    try {
      // Pending IL edits are written into the assembly before the member is added.
      const {edit_revision: revision} = await client.call('getEdits', {assembly})
      const result = await client.call('addMember', {assembly, revision, owner, member: {...member, name: member.name.trim()}})
      onAdded(result.token)
    } catch (e) {setError(diagnostic(e))} finally {setBusy(false)}
  }
  const field = 'w-full bg-background text-foreground border border-solid border-border rounded-[4px] py-[6px] px-2 font-mono text-[12px] [outline:none] focus:border-ring'
  return <dialog ref={dialog} className={cn(ui.dialog, "w-[min(560px,_calc(100vw_-_32px))]")} aria-labelledby="add-member-title" onCancel={e => {e.preventDefault(); onClose()}}>
    <header className={cn(ui.dialogHeader)}><div><h2 id="add-member-title">{kind === 'field' ? 'Add Field' : 'Add Method'}</h2><p>{ownerName} · {tokenHex(owner)}</p></div><button className={cn(ui.iconButton)} aria-label="Close add member" onClick={onClose}><X size={18} /></button></header>
    <form className="grid gap-3 px-6 pb-4 text-[12px] [&_label]:grid [&_label]:gap-1 [&_label>span]:text-muted-foreground" onSubmit={e => {e.preventDefault(); void submit()}}>
      <datalist id="add-member-types">{types.map(t => <option key={t} value={t} />)}</datalist>
      <label><span>Name</span><input className={field} aria-label="Member name" value={member.name} maxLength={1024} required autoFocus spellCheck={false} disabled={busy} onChange={e => update({name: e.target.value})} /></label>
      <label><span>{kind === 'field' ? 'Type' : 'Return type'}</span><input className={field} aria-label={kind === 'field' ? 'Field type' : 'Return type'} value={member.type} list="add-member-types" maxLength={512} required spellCheck={false} disabled={busy} onChange={e => update({type: e.target.value})} /></label>
      <div className="flex gap-3 items-end">
        <label className="flex-1"><span>Access</span><select className={field} aria-label="Access" value={member.access} disabled={busy} onChange={e => update({access: e.target.value as NewMemberRequest['access']})}><option value="public">public</option><option value="internal">internal</option><option value="protected">protected</option><option value="private">private</option></select></label>
        <label className="!flex items-center gap-2 pb-[6px]"><input type="checkbox" aria-label="Static" checked={member.static} disabled={busy} onChange={e => update({static: e.target.checked})} /><span>static</span></label>
      </div>
      {kind === 'method' && <fieldset className="border border-solid border-border rounded-[6px] py-2 px-3 [&_legend]:text-muted-foreground [&_legend]:px-1"><legend>Parameters</legend>
        {member.parameters.length === 0 && <p className="text-muted-foreground">No parameters.</p>}
        {member.parameters.map((p, i) => <div key={i} className="flex gap-2 py-1 items-center"><input className={field} aria-label={`Parameter ${i} type`} value={p.type} list="add-member-types" spellCheck={false} disabled={busy} onChange={e => setParameter(i, {type: e.target.value})} /><input className={field} aria-label={`Parameter ${i} name`} value={p.name} spellCheck={false} disabled={busy} onChange={e => setParameter(i, {name: e.target.value})} /><button type="button" className={cn(ui.iconButton)} aria-label={`Remove parameter ${i}`} disabled={busy} onClick={() => update({parameters: member.parameters.filter((_, j) => j !== i)})}><Trash2 size={14} /></button></div>)}
        <button type="button" className={cn(ui.button, ui.subtle, "mt-1")} disabled={busy || member.parameters.length >= 64} onClick={() => update({parameters: [...member.parameters, {name: `arg${member.parameters.length}`, type: 'int'}]})}><Plus size={14} />Add parameter</button>
      </fieldset>}
      <p className="text-muted-foreground leading-[1.6]">{kind === 'method' ? 'The new method returns a default value; edit its body with Edit Method Body (IL). ' : ''}Pending IL edits are written into the assembly first. Adding a member renumbers later field, method and parameter tokens; the workspace then shows the modified assembly.</p>
      {error && <div className="border border-solid border-warning-border text-warning-foreground bg-warning-background p-3 rounded-[6px] [overflow-wrap:anywhere]" role="alert"><strong>{error.message}</strong><p>{error.detail}</p></div>}
      <footer className="flex justify-end gap-2"><button type="button" className={cn(ui.button, ui.subtle)} onClick={onClose}>Cancel</button><button type="submit" className={cn(ui.button, ui.primary)} disabled={busy || !member.name.trim()}>{busy ? <LoaderCircle className="animate-spin motion-reduce:animate-none" size={14} /> : <Check size={14} />}{kind === 'field' ? 'Add field' : 'Add method'}</button></footer>
    </form>
  </dialog>
}
