import {test as base} from '@playwright/test'
export {expect,type Page} from '@playwright/test'
// Local-analysis suites explicitly disable network resolution. The dependency
// suite uses the real default and controlled official-SDK package responses.
export const test=base.extend<{frameworkDownloads:boolean}>({
  frameworkDownloads:[false,{option:true}],
  context:async({context,frameworkDownloads},use)=>{
    if(!frameworkDownloads) await context.addInitScript(()=>localStorage.setItem('ilens:framework-downloads','false'))
    await use(context)
  },
})
