import {createRoot} from 'react-dom/client'
import DecompilerApp from '../src/components/DecompilerApp'
import '../src/app/globals.css'

// Static-host entry only. The Rari/Docker entry continues to use src/app.
const root = document.getElementById('root')
if (!root) throw new Error('ILens root element is missing.')
createRoot(root).render(<DecompilerApp />)
