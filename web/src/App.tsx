import { CircleAlert, CircleCheck, Gauge, Info, Layers, MessageSquare, RadioTower, Search, Settings, SquareTerminal, Usb } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { SignalBars } from './components/ui'
import { cx } from './lib/style'
import { useLive } from './live-context'
import Cell from './pages/Cell'
import Console from './pages/Console'
import Dashboard from './pages/Dashboard'
import Esim from './pages/Esim'
import Messages from './pages/Messages'
import SettingsPage from './pages/Settings'
import Ussd from './pages/Ussd'

type Page = 'dashboard' | 'cell' | 'sms' | 'query' | 'esim' | 'console' | 'settings'

const PAGES: { id: Page; label: string; icon: ReactNode }[] = [
  { id: 'dashboard', label: '概览', icon: <Gauge className="size-4" /> },
  { id: 'cell', label: '小区', icon: <RadioTower className="size-4" /> },
  { id: 'sms', label: '短信', icon: <MessageSquare className="size-4" /> },
  { id: 'query', label: '查询', icon: <Search className="size-4" /> },
  { id: 'esim', label: 'eSIM', icon: <Layers className="size-4" /> },
  { id: 'console', label: 'AT 终端', icon: <SquareTerminal className="size-4" /> },
  { id: 'settings', label: '设置', icon: <Settings className="size-4" /> },
]

function usePage(): [Page, (p: Page) => void] {
  const read = () => {
    const h = location.hash.replace(/^#\/?/, '').split('/')[0] as Page
    return PAGES.some((p) => p.id === h) ? h : 'dashboard'
  }
  const [page, setPage] = useState<Page>(read)
  useEffect(() => {
    const onHash = () => setPage(read())
    window.addEventListener('hashchange', onHash)
    return () => window.removeEventListener('hashchange', onHash)
  }, [])
  return [page, (p) => (location.hash = `/${p}`)]
}

export default function App() {
  const [page, setPage] = usePage()
  const { connection, status, unread, socket, toasts } = useLive()

  return (
    <div className="flex min-h-dvh flex-col">
      <header className="sticky top-0 z-20 border-b border-zinc-200 bg-white/85 backdrop-blur dark:border-zinc-800 dark:bg-zinc-900/85">
        <div className="mx-auto flex max-w-6xl items-center gap-3 px-4 py-2.5">
          <div className="flex items-center gap-2 font-semibold">
            <span className="flex size-8 items-center justify-center rounded-lg bg-sky-600 text-white">
              <Usb className="size-4.5" />
            </span>
            <span className="hidden sm:inline">USB Card</span>
          </div>
          <div className="ml-auto flex min-w-0 items-center gap-3 text-sm">
            {status?.network.operator && (
              <span className="hidden truncate text-zinc-600 sm:inline dark:text-zinc-300">
                {status.sim.carrier ?? status.network.operator}
                {status.network.accessTech && <span className="ml-1 text-zinc-400">{status.network.accessTech}</span>}
              </span>
            )}
            {connection?.connected && <SignalBars level={status?.signal.level ?? 0} />}
            <span
              className={cx(
                'flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium',
                connection?.connected
                  ? 'bg-emerald-100 text-emerald-700 dark:bg-emerald-500/15 dark:text-emerald-300'
                  : 'bg-rose-100 text-rose-700 dark:bg-rose-500/15 dark:text-rose-300',
              )}
            >
              <span className={cx('size-1.5 rounded-full', connection?.connected ? 'bg-emerald-500' : 'bg-rose-500')} />
              {socket !== 'open' ? '服务器离线' : connection?.connected ? '模块在线' : '模块离线'}
            </span>
          </div>
        </div>
        <nav className="mx-auto flex max-w-6xl gap-1 overflow-x-auto px-3">
          {PAGES.map((p) => (
            <button
              key={p.id}
              onClick={() => setPage(p.id)}
              className={cx(
                'relative flex shrink-0 items-center gap-1.5 border-b-2 px-3 py-2 text-sm font-medium transition-colors',
                page === p.id
                  ? 'border-sky-600 text-sky-700 dark:border-sky-400 dark:text-sky-300'
                  : 'border-transparent text-zinc-500 hover:text-zinc-800 dark:text-zinc-400 dark:hover:text-zinc-200',
              )}
            >
              {p.icon}
              {p.label}
              {p.id === 'sms' && unread > 0 && (
                <span className="rounded-full bg-rose-500 px-1.5 text-[11px] leading-4 text-white">{unread}</span>
              )}
            </button>
          ))}
        </nav>
      </header>

      {socket === 'open' && connection && !connection.connected && (
        <div className="border-b border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900/50 dark:bg-amber-950/40 dark:text-amber-200">
          <div className="mx-auto flex max-w-6xl items-start gap-2 px-4 py-2 text-sm">
            <CircleAlert className="mt-0.5 size-4 shrink-0" />
            <div className="min-w-0">
              <div className="font-medium">未连接到 USB 模块，正在自动重试…</div>
              {connection.error && <div className="break-all text-xs opacity-80">{connection.error}</div>}
            </div>
          </div>
        </div>
      )}

      <main className="mx-auto w-full max-w-6xl flex-1 px-4 py-4">
        {page === 'dashboard' && <Dashboard />}
        {page === 'cell' && <Cell />}
        {page === 'sms' && <Messages />}
        {page === 'query' && <Ussd />}
        {page === 'esim' && <Esim />}
        {page === 'console' && <Console />}
        {page === 'settings' && <SettingsPage />}
      </main>

      <div className="pointer-events-none fixed inset-x-0 bottom-4 z-50 flex flex-col items-center gap-2 px-4">
        {toasts.map((t) => (
          <div
            key={t.id}
            className={cx(
              'pointer-events-auto flex max-w-md items-start gap-2 rounded-lg px-3 py-2 text-sm shadow-lg',
              t.kind === 'error'
                ? 'bg-rose-600 text-white'
                : t.kind === 'success'
                  ? 'bg-emerald-600 text-white'
                  : 'bg-zinc-800 text-white dark:bg-zinc-700',
            )}
          >
            {t.kind === 'error' ? (
              <CircleAlert className="mt-0.5 size-4 shrink-0" />
            ) : t.kind === 'success' ? (
              <CircleCheck className="mt-0.5 size-4 shrink-0" />
            ) : (
              <Info className="mt-0.5 size-4 shrink-0" />
            )}
            <span className="break-all">{t.text}</span>
          </div>
        ))}
      </div>
    </div>
  )
}
