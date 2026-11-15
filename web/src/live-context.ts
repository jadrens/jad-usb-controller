import { createContext, useCallback, useContext } from 'react'
import type { AtLogEntry, ConnectionState, ServerInfo, Sms, Status } from './api'

export interface SignalSample {
  t: number
  rsrp: number | null
  rssi: number | null
}

export interface Toast {
  id: number
  kind: 'info' | 'success' | 'error'
  text: string
}

export interface Live {
  socket: 'connecting' | 'open' | 'closed'
  connection: ConnectionState | null
  status: Status | null
  config: ServerInfo['config'] | null
  unread: number
  atLog: AtLogEntry[]
  signalHistory: SignalSample[]
  /** Bumped on every SMS event so message views can reload. */
  smsVersion: number
  lastSms: Sms | null
  toasts: Toast[]
  setStatus: (s: Status) => void
  refreshUnread: () => void
  clearAtLog: () => void
  notify: (text: string, kind?: Toast['kind']) => void
}

export const LiveContext = createContext<Live | null>(null)

export function useLive(): Live {
  const ctx = useContext(LiveContext)
  if (!ctx) throw new Error('useLive outside LiveProvider')
  return ctx
}

/** Run an async action, reporting failures as a toast. Returns undefined on error. */
export function useAction() {
  const { notify } = useLive()
  return useCallback(
    async <T,>(fn: () => Promise<T>, success?: string): Promise<T | undefined> => {
      try {
        const r = await fn()
        if (success) notify(success, 'success')
        return r
      } catch (e) {
        notify(e instanceof Error ? e.message : String(e), 'error')
        return undefined
      }
    },
    [notify],
  )
}
