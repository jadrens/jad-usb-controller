import { ArrowLeft, Inbox, MessageSquarePlus, RefreshCw, Send, Trash2 } from 'lucide-react'
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { api, type Conversation, type Sms } from '../api'
import { Button, CopyButton, Empty } from '../components/ui'
import { cx, inputClass } from '../lib/style'
import { formatTime, STORAGE_LABEL } from '../lib/format'
import { segmentInfo } from '../lib/sms'
import { useAction, useLive } from '../live-context'

const NEW = '\u0000new'

export default function Messages() {
  const { smsVersion, connection, refreshUnread, notify } = useLive()
  const run = useAction()
  const [convs, setConvs] = useState<Conversation[] | null>(null)
  const [peer, setPeer] = useState<string | null>(null)
  const [syncing, setSyncing] = useState(false)

  const loadConvs = useCallback(() => {
    api.conversations().then(setConvs).catch(() => setConvs([]))
  }, [])

  useEffect(loadConvs, [loadConvs, smsVersion])

  const sync = async () => {
    setSyncing(true)
    const r = await run(api.syncSms)
    setSyncing(false)
    if (r) {
      loadConvs()
      notify(r.new ? `收到 ${r.new} 条新短信` : '没有新短信', 'success')
    }
  }

  const removePeer = async (p: string) => {
    if (!confirm(`删除与 ${p} 的全部短信？仍存于 SIM 卡或模块中的也会一并删除。`)) return
    if (await run(() => api.deletePeer(p), '已删除')) {
      setPeer(null)
      loadConvs()
      refreshUnread()
    }
  }

  return (
    <div className="grid h-[calc(100dvh-8.5rem)] min-h-[420px] overflow-hidden rounded-xl border border-zinc-200 bg-white shadow-sm md:grid-cols-[300px_1fr] dark:border-zinc-800 dark:bg-zinc-900">
      <aside className={cx('flex min-h-0 flex-col border-zinc-200 md:border-r dark:border-zinc-800', peer !== null && 'hidden md:flex')}>
        <div className="flex items-center gap-2 border-b border-zinc-200 p-3 dark:border-zinc-800">
          <Button variant="primary" icon={<MessageSquarePlus className="size-4" />} onClick={() => setPeer(NEW)} className="flex-1">
            新短信
          </Button>
          <Button
            icon={<RefreshCw className="size-4" />}
            loading={syncing}
            disabled={!connection?.connected}
            onClick={sync}
            title="从模块读取短信"
          />
        </div>
        <ul className="min-h-0 flex-1 overflow-y-auto">
          {convs?.length === 0 && (
            <Empty icon={<Inbox className="size-8" />}>暂无短信</Empty>
          )}
          {convs?.map((c) => (
            <li key={c.peer}>
              <button
                onClick={() => setPeer(c.peer)}
                className={cx(
                  'flex w-full flex-col gap-0.5 border-b border-zinc-100 px-3 py-2.5 text-left hover:bg-zinc-50 dark:border-zinc-800/60 dark:hover:bg-zinc-800/50',
                  peer === c.peer && 'bg-sky-50 dark:bg-sky-500/10',
                )}
              >
                <div className="flex items-center gap-2">
                  <span className={cx('truncate font-mono text-sm', c.unread > 0 && 'font-semibold')}>{c.peer}</span>
                  {c.unread > 0 && <span className="rounded-full bg-rose-500 px-1.5 text-[11px] leading-4 text-white">{c.unread}</span>}
                  <span className="ml-auto shrink-0 text-xs text-zinc-400">{formatTime(c.lastTs)}</span>
                </div>
                <span className="line-clamp-2 text-xs text-zinc-500">
                  {c.lastDirection === 'out' && '我：'}
                  {c.lastText}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </aside>

      <div className={cx('min-h-0 flex-col', peer === null ? 'hidden md:flex' : 'flex')}>
        {peer === null ? (
          <Empty icon={<MessageSquarePlus className="size-8" />}>选择一个会话，或发送新短信</Empty>
        ) : (
          <Thread
            key={peer}
            peer={peer === NEW ? null : peer}
            onBack={() => setPeer(null)}
            onSent={(to) => {
              setPeer(to)
              loadConvs()
            }}
            onDelete={removePeer}
            onChanged={loadConvs}
          />
        )}
      </div>
    </div>
  )
}

function Thread({
  peer,
  onBack,
  onSent,
  onDelete,
  onChanged,
}: {
  peer: string | null
  onBack: () => void
  onSent: (to: string) => void
  onDelete: (peer: string) => void
  onChanged: () => void
}) {
  const { smsVersion, connection, refreshUnread, notify } = useLive()
  const run = useAction()
  const [messages, setMessages] = useState<Sms[]>([])
  const [to, setTo] = useState('')
  const [text, setText] = useState('')
  const [sending, setSending] = useState(false)
  const bottom = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!peer) return
    let cancelled = false
    api.messages(peer).then((m) => {
      if (cancelled) return
      setMessages(m.reverse())
      if (m.some((x) => x.status === 'unread')) {
        api.markRead(peer).then(() => {
          refreshUnread()
          onChanged()
        })
      }
    })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [peer, smsVersion])

  useLayoutEffect(() => {
    bottom.current?.scrollIntoView({ block: 'end' })
  }, [messages])

  const seg = segmentInfo(text)
  const target = peer ?? to.trim()
  const canSend = !!connection?.connected && !!text.trim() && /^\+?\d{3,20}$/.test(target) && !sending

  const send = async () => {
    if (!canSend) return
    setSending(true)
    const sms = await run(() => api.send(target, text))
    setSending(false)
    if (sms) {
      setText('')
      if (sms.status === 'failed') notify(`发送失败：${sms.error ?? ''}`, 'error')
      onSent(target)
    }
  }

  const removeOne = async (id: number) => {
    const m = messages.find((x) => x.id === id)
    const where = m?.onModem && m.storage ? `（同时从${STORAGE_LABEL[m.storage] ?? m.storage}中删除）` : ''
    if (!confirm(`删除这条短信？${where}`)) return
    if (await run(() => api.deleteSms(id))) {
      setMessages((m) => m.filter((x) => x.id !== id))
      onChanged()
    }
  }

  return (
    <>
      <div className="flex items-center gap-2 border-b border-zinc-200 px-3 py-2 dark:border-zinc-800">
        <Button variant="ghost" className="md:hidden" icon={<ArrowLeft className="size-4" />} onClick={onBack} />
        {peer ? (
          <>
            <span className="font-mono font-semibold">{peer}</span>
            <CopyButton text={peer} />
            <Button
              variant="ghost"
              className="ml-auto text-rose-600!"
              icon={<Trash2 className="size-4" />}
              onClick={() => onDelete(peer)}
            >
              删除会话
            </Button>
          </>
        ) : (
          <input
            autoFocus
            className={inputClass + ' font-mono'}
            placeholder="收件人号码，如 10086 或 +8613800138000"
            value={to}
            onChange={(e) => setTo(e.target.value.replace(/[^\d+]/g, ''))}
          />
        )}
      </div>

      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto bg-zinc-50 p-4 dark:bg-zinc-950/40">
        {messages.map((m) => (
          <div key={m.id} className={cx('group flex flex-col', m.direction === 'out' ? 'items-end' : 'items-start')}>
            <div
              className={cx(
                'max-w-[85%] rounded-2xl px-3.5 py-2 text-sm break-words whitespace-pre-wrap shadow-sm sm:max-w-[70%]',
                m.direction === 'out'
                  ? 'rounded-br-md bg-sky-600 text-white'
                  : 'rounded-bl-md bg-white text-zinc-900 dark:bg-zinc-800 dark:text-zinc-100',
                m.status === 'failed' && 'bg-rose-600!',
              )}
            >
              {m.text}
            </div>
            <div className="mt-1 flex items-center gap-2 px-1 text-[11px] text-zinc-400">
              <span>{formatTime(m.ts)}</span>
              {m.parts > 1 && <span>{m.parts} 段</span>}
              {m.storage && (
                <span
                  title={m.onModem ? '仍保存在模块存储中' : '已转存到本地数据库'}
                  className={cx(
                    'rounded px-1 py-px',
                    m.onModem
                      ? 'bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300'
                      : 'bg-zinc-200/70 text-zinc-500 dark:bg-zinc-800',
                  )}
                >
                  {m.onModem ? '存于' : '来自'}
                  {STORAGE_LABEL[m.storage] ?? m.storage}
                </span>
              )}
              {m.direction === 'out' && (
                <span className={cx(m.status === 'failed' && 'text-rose-500')}>
                  {m.status === 'sending' ? '发送中…' : m.status === 'sent' ? '已发送' : `失败 ${m.error ?? ''}`}
                </span>
              )}
              <button className="opacity-0 transition-opacity group-hover:opacity-100 hover:text-rose-500" onClick={() => removeOne(m.id)}>
                <Trash2 className="size-3" />
              </button>
            </div>
          </div>
        ))}
        <div ref={bottom} />
      </div>

      <form
        className="border-t border-zinc-200 p-3 dark:border-zinc-800"
        onSubmit={(e) => {
          e.preventDefault()
          send()
        }}
      >
        <div className="flex items-end gap-2">
          <textarea
            rows={2}
            className={inputClass + ' resize-none'}
            placeholder={connection?.connected ? '输入短信内容，Ctrl+Enter 发送' : '模块离线，无法发送'}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
                e.preventDefault()
                send()
              }
            }}
          />
          <Button variant="primary" type="submit" loading={sending} disabled={!canSend} icon={<Send className="size-4" />}>
            发送
          </Button>
        </div>
        <div className="mt-1 text-right text-[11px] text-zinc-400">
          {seg.encoding} · {seg.units} 字符 · {seg.segments} 条 · 本条剩余 {seg.remaining}
        </div>
      </form>
    </>
  )
}
