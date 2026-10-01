'use client'
export type CodeView = 'C#' | 'IL' | 'Metadata' | 'Hex'
export interface Location {assembly: number; token: number; view: CodeView}
export interface Navigation {tabs: Location[]; history: Location[]; index: number; bookmarks: Location[]}
export const emptyNavigation: Navigation = {tabs:[], history:[], index:-1, bookmarks:[]}
export const locationKey = (location: Pick<Location, 'assembly' | 'token'>) => `${location.assembly}:${location.token}`
export function visit(state: Navigation, location: Location): Navigation {
  const key = locationKey(location)
  const tabs = state.tabs.some(t => locationKey(t) === key) ? state.tabs.map(t => locationKey(t) === key ? location : t) : [...state.tabs.slice(-31), location]
  const current = state.history[state.index]
  if (current && locationKey(current) === key) {
    const history = [...state.history]; history[state.index] = location
    return {...state, tabs, history}
  }
  const history = [...state.history.slice(0, state.index + 1), location].slice(-128)
  return {...state, tabs, history, index:history.length - 1}
}
export function travel(state: Navigation, direction: -1 | 1): Navigation {
  const index = Math.min(state.history.length - 1, Math.max(0, state.index + direction))
  const location = state.history[index]
  if (!location) return state
  const tabs = state.tabs.some(t => locationKey(t) === locationKey(location)) ? state.tabs.map(t => locationKey(t) === locationKey(location) ? location : t) : [...state.tabs.slice(-31), location]
  return {...state, tabs, index}
}
export function toggleBookmark(state: Navigation, location: Location): Navigation {
  const present = state.bookmarks.some(t => locationKey(t) === locationKey(location))
  return {...state, bookmarks: present ? state.bookmarks.filter(t => locationKey(t) !== locationKey(location)) : [...state.bookmarks.slice(-63), location]}
}
