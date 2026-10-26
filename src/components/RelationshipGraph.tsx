'use client'
import {useEffect,useMemo,useRef,useState} from 'react'
import type {FunctionMap,MappedFunction} from '../lib/types'

// The whole assembly as one map. Clicking a node focuses it: only that node, what it
// connects to and (for a type) its own functions stay on screen. Searching keeps the
// matches — by type name, function name or a string literal the function loads — and
// their neighbours. Built from decoded CIL; nothing executes.
type Kind='inherits'|'implements'|'uses'|'calls'|'contains'
type Link={from:number;to:number;kind:Kind;weight:number}
type Pos={x:number;y:number}
type GNode={id:number;type:boolean;label:string;sub:string;owner?:number;accent:string}
const MAX_NODES=420
const FUNCTIONS_PER_TYPE=60
const edgeStyle:Record<Kind,{label:string;cls:string;dash?:string}>={
  inherits:{label:'inherits',cls:'stroke-[var(--primary)]'},
  implements:{label:'implements',cls:'stroke-[#6ea8fe]',dash:'5 4'},
  uses:{label:'has field of',cls:'stroke-[var(--muted-foreground)]',dash:'2 4'},
  calls:{label:'calls',cls:'stroke-[var(--muted-foreground)]'},
  contains:{label:'contains',cls:'stroke-[var(--border)]'},
}
const accents:Record<string,string>={interface:'#6ea8fe',enum:'#d8a657',struct:'var(--primary)',delegate:'#a9a1e1'}
const accentOf=(kind:string)=>accents[kind]??'var(--foreground)'
const short=(s:string,n:number)=>s.length>n?s.slice(0,n-1)+'…':s

export default function RelationshipGraph({map,onOpen}:{map:FunctionMap;onOpen:(token:number)=>void}) {
  const svgRef=useRef<SVGSVGElement>(null)
  const pos=useRef(new Map<number,Pos>())
  const [,tick]=useState(0)
  const drag=useRef<{id:number|null;panX?:number;panY?:number;moved?:boolean}|null>(null)
  const [view,setView]=useState({x:-500,y:-500,w:1000,h:1000})
  const [expanded,setExpanded]=useState<Set<number>>(new Set())
  const [focus,setFocus]=useState<number|null>(null)
  const [query,setQuery]=useState('')

  const functionsByOwner=useMemo(()=>{
    const by=new Map<number,MappedFunction[]>()
    for(const f of map.functions){const list=by.get(f.owner);if(list)list.push(f);else by.set(f.owner,[f])}
    return by
  },[map])
  const typeByToken=useMemo(()=>new Map(map.types.map(t=>[t.token,t])),[map])

  const {nodes,links,matchCount}=useMemo(()=>{
    const q=query.trim().toLowerCase()
    const hitType=(t:FunctionMap['types'][number])=>!!q&&q!=='unused'&&(t.name.toLowerCase().includes(q)||t.namespace.toLowerCase().includes(q)||t.full_name.toLowerCase().includes(q))
    // Searching also looks inside the string literals each function loads, so a word
    // from a message finds the code that uses it.
    // `unused` is the one keyword search: it answers "what has no caller here?",
    // which is as far as a static scan can go — reflection is invisible to it.
    const hitFn=(f:MappedFunction)=>!!q&&(q==='unused'?f.tags.includes('unreferenced'):f.name.toLowerCase().includes(q)||f.signature.toLowerCase().includes(q)||f.strings.some(s=>s.toLowerCase().includes(q)))

    const fnVisible=new Set<number>()
    for(const owner of expanded)for(const f of (functionsByOwner.get(owner)??[]).slice(0,FUNCTIONS_PER_TYPE))fnVisible.add(f.token)
    const matched=new Set<number>()
    if(q){
      for(const t of map.types)if(hitType(t))matched.add(t.token)
      for(const f of map.functions)if(hitFn(f)){matched.add(f.token);fnVisible.add(f.token)}
    }

    const all:GNode[]=[]
    for(const t of map.types)all.push({id:t.token,type:true,label:short(t.name,22),sub:`${t.kind} · ${t.members}`,accent:accentOf(t.kind)})
    for(const f of map.functions)if(fnVisible.has(f.token))all.push({id:f.token,type:false,label:short(f.name,24),sub:`${f.size} B`,owner:f.owner,accent:'var(--muted-foreground)'})
    const present=new Set(all.map(n=>n.id))

    const edges=new Map<string,Link>()
    const add=(from:number,to:number,kind:Kind,weight=1)=>{
      if(from===to||!present.has(from)||!present.has(to))return
      const key=`${from}-${to}-${kind}`
      const e=edges.get(key)
      if(e)e.weight+=weight;else edges.set(key,{from,to,kind,weight})
    }
    for(const t of map.types){
      if(t.base)add(t.token,t.base.token,'inherits')
      for(const i of t.interfaces)add(t.token,i.token,'implements')
      for(const u of t.uses)add(t.token,u.token,'uses')
    }
    const ownerOf=new Map(map.functions.map(f=>[f.token,f.owner]))
    for(const f of map.functions){
      if(fnVisible.has(f.token))add(f.owner,f.token,'contains',4)
      const from=fnVisible.has(f.token)?f.token:f.owner
      for(const c of f.calls){
        const owner=ownerOf.get(c)
        if(owner!==undefined)add(from,fnVisible.has(c)?c:owner,'calls')
      }
    }
    const everyLink=[...edges.values()]

    // Focus and search both keep their seeds plus whatever those connect to.
    const seeds=q?matched:focus!==null&&present.has(focus)?new Set([focus]):null
    let keep:Set<number>|null=null
    if(seeds){
      keep=new Set(seeds)
      for(const l of everyLink){
        if(seeds.has(l.from))keep.add(l.to)
        if(seeds.has(l.to))keep.add(l.from)
      }
    }
    const kept=keep
    const nodes=(kept?all.filter(n=>kept.has(n.id)):all).slice(0,MAX_NODES)
    const live=new Set(nodes.map(n=>n.id))
    return {nodes,links:everyLink.filter(l=>live.has(l.from)&&live.has(l.to)),matchCount:q?matched.size:0}
  },[map,expanded,focus,query,functionsByOwner])

  // Fruchterman–Reingold; nodes that stay keep their place so changes are smooth.
  useEffect(()=>{
    const n=nodes.length
    if(!n)return
    const previous=pos.current
    const p=new Map<number,Pos>()
    let seed=(n*2654435761)>>>0
    const rnd=()=>{seed=(seed*1664525+1013904223)>>>0;return seed/4294967296}
    let fresh=0
    for(const node of nodes){
      const had=previous.get(node.id)
      if(had)p.set(node.id,{...had})
      else{
        fresh++
        const anchor=node.owner!==undefined?previous.get(node.owner):undefined
        p.set(node.id,anchor?{x:anchor.x+(rnd()-0.5)*160,y:anchor.y+(rnd()-0.5)*160}:{x:(rnd()-0.5)*700,y:(rnd()-0.5)*700})
      }
    }
    pos.current=p
    const k=Math.min(Math.sqrt(1_000_000/n),220),k2=k*k
    const whole=fresh===n
    const iters=whole?(n>160?260:420):150
    let temp=whole?120:60
    const disp=new Map<number,Pos>()
    for(let s=0;s<iters;s++){
      for(const node of nodes)disp.set(node.id,{x:0,y:0})
      for(let i=0;i<n;i++)for(let j=i+1;j<n;j++){
        const a=p.get(nodes[i].id)!,b=p.get(nodes[j].id)!
        let dx=a.x-b.x,dy=a.y-b.y;const d=Math.hypot(dx,dy)||0.01
        const rep=k2/d;dx=dx/d*rep;dy=dy/d*rep
        const di=disp.get(nodes[i].id)!,dj=disp.get(nodes[j].id)!
        di.x+=dx;di.y+=dy;dj.x-=dx;dj.y-=dy
      }
      for(const l of links){
        const a=p.get(l.from)!,b=p.get(l.to)!
        let dx=a.x-b.x,dy=a.y-b.y;const d=Math.hypot(dx,dy)||0.01
        const att=(d*d/k)*Math.min(0.5+l.weight*0.2,2.5);dx=dx/d*att;dy=dy/d*att
        const da=disp.get(l.from)!,db=disp.get(l.to)!
        da.x-=dx;da.y-=dy;db.x+=dx;db.y+=dy
      }
      for(const node of nodes){
        const pt=p.get(node.id)!,dp=disp.get(node.id)!
        // Gravity must balance the repulsion sum or loose nodes fly off the canvas.
        dp.x-=pt.x*0.6;dp.y-=pt.y*0.6
        const d=Math.hypot(dp.x,dp.y)||0.01,m=Math.min(d,temp)
        pt.x+=dp.x/d*m;pt.y+=dp.y/d*m
      }
      temp=Math.max(temp*0.975,2)
    }
    const xs=[...p.values()].map(v=>v.x).sort((a,b)=>a-b)
    const ys=[...p.values()].map(v=>v.y).sort((a,b)=>a-b)
    const pick=(arr:number[],f:number)=>arr[Math.min(arr.length-1,Math.max(0,Math.floor(arr.length*f)))]
    let minX=pick(xs,0.025),maxX=pick(xs,0.975),minY=pick(ys,0.025),maxY=pick(ys,0.975)
    if(maxX-minX<200){minX-=150;maxX+=150}
    if(maxY-minY<200){minY-=150;maxY+=150}
    const pad=90
    setView({x:minX-pad,y:minY-pad,w:maxX-minX+pad*2,h:maxY-minY+pad*2})
    tick(t=>t+1)
  },[nodes,links])

  function toSvg(e:{clientX:number;clientY:number}){
    const svg=svgRef.current;if(!svg)return {x:0,y:0}
    const pt=svg.createSVGPoint();pt.x=e.clientX;pt.y=e.clientY
    const m=svg.getScreenCTM();return m?pt.matrixTransform(m.inverse()):{x:0,y:0}
  }
  const down=(e:React.PointerEvent,id:number|null)=>{
    (e.target as Element).setPointerCapture?.(e.pointerId)
    if(id!==null)drag.current={id,moved:false}
    else{const s=toSvg(e);drag.current={id:null,panX:s.x,panY:s.y,moved:false}}
  }
  const move=(e:React.PointerEvent)=>{
    const d=drag.current;if(!d)return
    const s=toSvg(e)
    if(d.id!==null){d.moved=true;pos.current.set(d.id,{x:s.x,y:s.y});tick(t=>t+1)}
    else{d.moved=true;setView(v=>({...v,x:v.x-(s.x-d.panX!),y:v.y-(s.y-d.panY!)}))}
  }
  const up=()=>{drag.current=null}
  const wheel=(e:React.WheelEvent)=>{
    const s=toSvg(e),f=e.deltaY>0?1.12:0.89
    setView(v=>({x:s.x-(s.x-v.x)*f,y:s.y-(s.y-v.y)*f,w:v.w*f,h:v.h*f}))
  }
  const activate=(node:GNode)=>{
    setFocus(node.id)
    if(node.type)setExpanded(prev=>new Set(prev).add(node.id))
  }
  const showAll=()=>{setFocus(null);setExpanded(new Set());setQuery('')}
  const focusLabel=focus===null?null:typeByToken.get(focus)?.name??map.functions.find(f=>f.token===focus)?.name??null

  return <div className="relative h-full w-full overflow-hidden bg-[var(--surface)]">
    <div className="absolute left-2 top-2 z-1 flex max-w-[calc(100%-1rem)] flex-wrap items-center gap-2">
      <input className="h-8 w-64 rounded-md border border-solid border-border bg-[var(--popover)] px-2.5 text-[12px]" type="search"
        placeholder="Search types, functions, strings or “unused”…" aria-label="Search the map" value={query} onChange={e=>setQuery(e.target.value)}/>
      {!!query.trim()&&<span className="rounded-md bg-[var(--popover)]/90 px-2 py-1 text-[11px] text-muted-foreground">{matchCount} match{matchCount===1?'':'es'}</span>}
      {focusLabel&&!query.trim()&&<span className="rounded-md bg-[var(--popover)]/90 px-2 py-1 text-[11px] text-muted-foreground">Focused on <strong className="font-mono text-foreground">{focusLabel}</strong></span>}
      {(focus!==null||!!query.trim()||expanded.size>0)&&<button className="h-8 rounded-md border border-solid border-border bg-[var(--popover)] px-2.5 text-[11px] hover:bg-hover" onClick={showAll}>Show whole map</button>}
    </div>
    {!nodes.length
      ?<p className="flex h-full items-center justify-center text-[12px] text-muted-foreground">{query.trim()?`Nothing matches “${query.trim()}”.`:'This assembly declares no types.'}</p>
      :<svg ref={svgRef} viewBox={`${view.x} ${view.y} ${view.w} ${view.h}`} className="h-full w-full touch-none select-none"
        role="img" aria-label="Assembly relationship map"
        onPointerDown={e=>down(e,null)} onPointerMove={move} onPointerUp={up} onPointerLeave={up} onWheel={wheel}
        onClick={()=>{if(!drag.current?.moved)setFocus(null)}}>
        <defs>
          {(['calls','inherits'] as Kind[]).map(k=>
            <marker key={k} id={`rm-arrow-${k}`} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
              <path d="M0,0 L10,5 L0,10 z" className={`${edgeStyle[k].cls} fill-current`} fillOpacity={0.55}/>
            </marker>)}
        </defs>
        {links.map((l,i)=>{
          const a=pos.current.get(l.from),b=pos.current.get(l.to)
          if(!a||!b)return null
          const st=edgeStyle[l.kind]
          return <line key={i} x1={a.x} y1={a.y} x2={b.x} y2={b.y} className={st.cls}
            strokeWidth={l.kind==='calls'?Math.min(0.6+l.weight*0.25,3):l.kind==='contains'?0.7:1.4}
            strokeDasharray={st.dash} markerEnd={l.kind==='calls'||l.kind==='inherits'?`url(#rm-arrow-${l.kind})`:undefined}
            opacity={l.kind==='contains'?0.35:l.kind==='calls'?0.45:0.85}/>
        })}
        {nodes.map(node=>{
          const p=pos.current.get(node.id);if(!p)return null
          const w=node.type?150:122,h=node.type?34:22
          const sel=node.id===focus
          return <g key={node.id} role="button" tabIndex={0}
            aria-label={node.type?`${node.label} (${node.sub.split(' · ')[0]})`:`function ${node.label}`}
            transform={`translate(${p.x-w/2},${p.y-h/2})`}
            className="cursor-pointer [&:focus-visible_rect]:stroke-[var(--ring)]"
            onPointerDown={e=>{e.stopPropagation();down(e,node.id)}}
            onClick={e=>{e.stopPropagation();if(!drag.current?.moved)activate(node)}}
            onKeyDown={e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();activate(node)}}}
            onDoubleClick={e=>{e.stopPropagation();onOpen(node.id)}}>
            <title>{node.type?`${node.label} · ${node.sub} · click to focus, double-click to open`:`${node.label} · ${node.sub} · double-click to open`}</title>
            <rect width={w} height={h} rx={node.type?7:5} className={node.type?'fill-[var(--surface-raised)]':'fill-[var(--popover)]'}
              stroke={sel?'var(--ring)':'var(--border)'} strokeWidth={sel?2.5:1}/>
            <rect width={node.type?4:3} height={h} rx={2} fill={node.accent}/>
            {node.type
              ?<><text x={w/2+2} y={13} dominantBaseline="middle" textAnchor="middle" className="fill-[var(--foreground)] font-mono text-[11px]">{node.label}</text>
                 <text x={w/2+2} y={25} dominantBaseline="middle" textAnchor="middle" className="fill-[var(--muted-foreground)] text-[8px]">{node.sub}</text></>
              :<text x={w/2+2} y={h/2} dominantBaseline="middle" textAnchor="middle" className="fill-[var(--muted-foreground)] font-mono text-[9px]">{node.label}</text>}
          </g>
        })}
      </svg>}
    <div className="pointer-events-none absolute bottom-2 left-2 flex flex-wrap gap-x-3 gap-y-1 rounded-md bg-[var(--popover)]/85 px-2.5 py-1.5 text-[10px] text-muted-foreground">
      {(Object.keys(edgeStyle) as Kind[]).map(k=>
        <span key={k} className="flex items-center gap-1">
          <svg width="18" height="6" aria-hidden="true"><line x1="0" y1="3" x2="18" y2="3" className={edgeStyle[k].cls} strokeWidth={1.5} strokeDasharray={edgeStyle[k].dash}/></svg>{edgeStyle[k].label}
        </span>)}
      <span>· click a node to focus it · double-click to open · scroll to zoom, drag to pan</span>
    </div>
    {nodes.length>=MAX_NODES&&<p className="absolute right-2 top-2 rounded bg-[var(--popover)]/85 px-2 py-1 text-[10px] text-muted-foreground">Showing the first {MAX_NODES} nodes</p>}
  </div>
}
