import type { LayoutProps, Metadata } from 'rari'
// Vite's global Tailwind entry is linked in index.html; rariStyles attaches
// the compiled, hashed stylesheet to this layout's production manifest.
export default function RootLayout({children}: LayoutProps) { return <>{children}</> }
export const metadata: Metadata = {title: 'ILens — .NET decompiler', description: 'Explore .NET assemblies, inspect metadata and IL, and reconstruct C#.'}
