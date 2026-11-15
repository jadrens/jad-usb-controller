import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { api, type AtLogEntry, type ConnectionState, type ServerEvent, type ServerInfo, type Sms, type Status } from './api'
import { LiveContext, type Live, type SignalSample, type Toast } from './live-context'

const MAX_LOG = 1500
const MAX_SAMPLES = 180

export function LiveProvider({ children }: { children: ReactNode }) {
  const [socket, setSocket] = useState<Live['socket']>('connecting')
  const [connection, setConnection] = useState<ConnectionState | null>(null)
  const [status, setStatusState] = useState<Status | null>(null)
  const [config, setConfig] = useState<ServerInfo['config'] | null>(null)
  const [unread, setUnread] = useState(0)
  const [atLog, setAtLog] = useState<AtLogEntry[]>([])
  const [signalHistory, setSignalHistory] = useState<SignalSample[]>([])
  const [smsVersion, setSmsVersion] = useState(0)
  const [lastSms, setLastSms] = useState<Sms | null>(null)
  const [toasts, setToasts] = useState<Toast[]>([])
  const toastId = useRef(0)

  const notify = useCallback((text: string, kind: Toast['kind'] = 'info') => {
    const id = ++toastId.current
    setToasts((t) => [...t.slice(-3), { id, kind, text }])
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), kind === 'error' ? 6000 : 3500)
  }, [])

  const setStatus = useCallback((s: Status) => {
    setStatusState(s)
    if (s.updatedAt) {
      const t = Date.parse(s.updatedAt)
      setSignalHistory((h) => {
        if (h.length && h[h.length - 1].t === t) return h
        return [...h, { t, rsrp: s.signal.rsrp, rssi: s.signal.rssi }].slice(-MAX_SAMPLES)
      })
    }
  }, [])

  const refreshUnread = useCallback(() => {
    api
      .info()
      .then((i) => {
        setUnread(i.unread)
        setConfig(i.config)
      })
      .catch(() => {})
  }, [])

  useEffect(() => {
    api
      .info()
      .then((i) => {
        setConnection(i.connection)
        setStatus(i.status)
        setUnread(i.unread)
        setConfig(i.config)
      })
      .catch(() => {})
  }, [setStatus])

  // keep the latest callbacks reachable from the long-lived socket handler
  const handlers = useRef({ setStatus, refreshUnread, notify })
  useEffect(() => {
    handlers.current = { setStatus, refreshUnread, notify }
  }, [setStatus, refreshUnread, notify])

  useEffect(() => {
    let ws: WebSocket | null = null
    let retry: ReturnType<typeof setTimeout> | undefined
    let delay = 1000
    let stopped = false

    const connect = () => {
      setSocket('connecting')
      const proto = location.protocol === 'https:' ? 'wss' : 'ws'
      ws = new WebSocket(`${proto}://${location.host}/api/ws`)
      ws.onopen = () => {
        delay = 1000
        setSocket('open')
        handlers.current.refreshUnread()
      }
      ws.onmessage = (e) => {
        const ev = JSON.parse(e.data) as ServerEvent
        switch (ev.type) {
          case 'status':
            handlers.current.setStatus(ev.status)
            break
          case 'connection':
            setConnection(ev.connection)
            break
          case 'sms':
            setLastSms(ev.sms)
            setSmsVersion((v) => v + 1)
            handlers.current.refreshUnread()
            if (ev.sms.direction === 'in') {
              handlers.current.notify(`新短信 · ${ev.sms.peer}：${ev.sms.text.slice(0, 40)}`)
            }
            break
          case 'sms_sync':
            setSmsVersion((v) => v + 1)
            break
          case 'at_log': {
            const { dir, line, ts } = ev
            setAtLog((l) => (l.length >= MAX_LOG ? [...l.slice(-MAX_LOG + 200), { dir, line, ts }] : [...l, { dir, line, ts }]))
            break
          }
        }
      }
      ws.onclose = () => {
        setSocket('closed')
        if (!stopped) {
          retry = setTimeout(connect, delay)
          delay = Math.min(delay * 2, 15000)
        }
      }
    }
    connect()
    return () => {
      stopped = true
      clearTimeout(retry)
      ws?.close()
    }
  }, [])

  const clearAtLog = useCallback(() => setAtLog([]), [])

  return (
    <LiveContext.Provider
      value={{
        socket,
        connection,
        status,
        config,
        unread,
        atLog,
        signalHistory,
        smsVersion,
        lastSms,
        toasts,
        setStatus,
        refreshUnread,
        clearAtLog,
        notify,
      }}
    >
      {children}
    </LiveContext.Provider>
  )
}
