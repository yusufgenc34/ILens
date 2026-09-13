import {twMerge} from 'tailwind-merge'
export function cn(...classes: (string | false | null | undefined)[]) {return twMerge(classes.filter(Boolean).join(' '))}

// Shared Tailwind recipes for repeated controls and dialog structure.
export const ui = {
  button: "button inline-flex items-center justify-center gap-2 border border-solid border-border bg-surface-raised whitespace-nowrap rounded-md text-[13px] font-medium h-9 py-0 px-3 hover:bg-hover [&_kbd]:ml-1.5",
  primary: "open-button text-primary-foreground bg-primary border-primary hover:bg-primary-hover hover:border-primary-hover [&_kbd]:text-inherit [&_kbd]:border-current [&_kbd]:opacity-70 [&_kbd]:bg-transparent",
  subtle: "subtle bg-transparent border-transparent text-foreground",
  iconButton: "icon-button bg-transparent border-0 inline-flex items-center justify-center text-muted-foreground rounded-md p-2 hover:text-foreground hover:bg-hover [&.enabled]:text-foreground [&.enabled]:bg-hover",
  dialog: "settings-dialog w-[min(620px,_92vw)] max-h-[90dvh] overflow-auto p-0 text-foreground bg-popover border border-solid border-border rounded-[8px] shadow-[var(--shadow)] font-sans text-[13px] font-normal leading-normal scheme-inherit backdrop:bg-[#0009] m-auto",
  dialogHeader: "settings-header flex items-start justify-between gap-4 pt-5.5 pr-6 pb-4.5 pl-6 border-b border-solid border-b-border [&_h2]:flex [&_h2]:items-center [&_h2]:gap-2.5 [&_h2]:text-[16px] [&_h2]:font-semibold [&_h2]:mt-0 [&_h2]:mr-0 [&_h2]:mb-1.5 [&_h2]:ml-0 [&_p]:text-muted-foreground [&_p]:text-[12px] [&_p]:m-0",
  dialogBody: "settings-body pt-2 pr-6 pb-6 pl-6",
  dialogFooter: "settings-footer flex justify-between gap-4 py-4 px-6 border-t border-solid border-t-border",
  exportDialog: "export-dialog w-[min(680px,_calc(100vw_-_40px))] [&[open]]:flex [&[open]]:flex-col [&[open]]:overflow-hidden [&_>_.settings-header]:shrink-0 [&_>_.settings-footer]:shrink-0 [&_>_.settings-body]:min-h-0 [&_>_.settings-body]:overflow-auto",
  exportBody: "export-body flex flex-col gap-5 [&_section_+_section]:border-t [&_section_+_section]:border-solid [&_section_+_section]:border-t-border [&_section_+_section]:pt-5 [&_h3]:mt-0 [&_h3]:mr-0 [&_h3]:mb-2 [&_h3]:ml-0 [&_h3]:text-[13px] [&_h3]:font-semibold [&_p]:text-muted-foreground [&_p]:text-[12px] [&_p]:leading-[1.65] [&_p]:my-2 [&_p]:mx-0 [&_p]:[overflow-wrap:anywhere]",
  actions: "export-actions flex items-center gap-2 flex-wrap",
}
