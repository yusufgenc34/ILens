import type { LayoutProps, Metadata } from 'rari'
import './globals.module.css'
export default function RootLayout({children}: LayoutProps) { return <>{children}</> }
export const metadata: Metadata = {title: 'ILens — .NET decompiler', description: 'Explore .NET assemblies, inspect metadata and IL, and reconstruct C#.'}
