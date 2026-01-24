import { useState, useEffect, useCallback } from 'react'
import '@copilotkit/react-ui/styles.css'

// Configuration for connecting to Everruns
const API_BASE = '/v1'
const DEFAULT_ORG = 'org_00000000000000000000000000000001'

interface Agent {
  id: string
  name: string
}

interface Session {
  id: string
}

interface AgUiEvent {
  type: string
  threadId?: string
  runId?: string
  messageId?: string
  delta?: string
  message?: string
  code?: string
  toolCallId?: string
  toolCallName?: string
  result?: string
  timestamp?: number
}

function App() {
  const [agent, setAgent] = useState<Agent | null>(null)
  const [session, setSession] = useState<Session | null>(null)
  const [messages, setMessages] = useState<Array<{ role: string; content: string }>>([])
  const [input, setInput] = useState('')
  const [isLoading, setIsLoading] = useState(false)
  const [events, setEvents] = useState<AgUiEvent[]>([])
  const [currentText, setCurrentText] = useState('')
  const [error, setError] = useState<string | null>(null)

  // Initialize agent and session on mount
  useEffect(() => {
    async function init() {
      try {
        // Create agent
        const agentRes = await fetch(`${API_BASE}/orgs/${DEFAULT_ORG}/agents`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            name: 'CopilotKit Demo Agent',
            system_prompt: 'You are a helpful assistant. Be concise and friendly.',
          }),
        })
        if (!agentRes.ok) throw new Error('Failed to create agent')
        const agentData = await agentRes.json()
        setAgent(agentData)

        // Create session
        const sessionRes = await fetch(
          `${API_BASE}/orgs/${DEFAULT_ORG}/agents/${agentData.id}/sessions`,
          {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({}),
          }
        )
        if (!sessionRes.ok) throw new Error('Failed to create session')
        const sessionData = await sessionRes.json()
        setSession(sessionData)
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Initialization failed')
      }
    }
    init()
  }, [])

  // Subscribe to AG-UI SSE events
  const subscribeToEvents = useCallback((agentId: string, sessionId: string) => {
    const eventSource = new EventSource(
      `${API_BASE}/orgs/${DEFAULT_ORG}/agents/${agentId}/sessions/${sessionId}/ag-ui/sse`
    )

    eventSource.onopen = () => {
      console.log('AG-UI SSE connected')
    }

    eventSource.onerror = (err) => {
      console.error('AG-UI SSE error:', err)
      eventSource.close()
    }

    // Handle AG-UI events
    const handleEvent = (event: MessageEvent) => {
      try {
        const data: AgUiEvent = JSON.parse(event.data)
        setEvents((prev) => [...prev.slice(-50), data]) // Keep last 50 events

        switch (data.type) {
          case 'TEXT_MESSAGE_START':
            setCurrentText('')
            break
          case 'TEXT_MESSAGE_CONTENT':
            if (data.delta) {
              setCurrentText((prev) => prev + data.delta)
            }
            break
          case 'TEXT_MESSAGE_END':
            setMessages((prev) => [
              ...prev,
              { role: 'assistant', content: currentText || '(empty response)' },
            ])
            setCurrentText('')
            break
          case 'RUN_FINISHED':
            setIsLoading(false)
            break
          case 'RUN_ERROR':
            setIsLoading(false)
            setError(data.message || 'Unknown error')
            break
        }
      } catch (err) {
        console.error('Failed to parse event:', err)
      }
    }

    // Subscribe to specific AG-UI event types
    eventSource.addEventListener('RUN_STARTED', handleEvent)
    eventSource.addEventListener('RUN_FINISHED', handleEvent)
    eventSource.addEventListener('RUN_ERROR', handleEvent)
    eventSource.addEventListener('TEXT_MESSAGE_START', handleEvent)
    eventSource.addEventListener('TEXT_MESSAGE_CONTENT', handleEvent)
    eventSource.addEventListener('TEXT_MESSAGE_END', handleEvent)
    eventSource.addEventListener('TOOL_CALL_START', handleEvent)
    eventSource.addEventListener('TOOL_CALL_RESULT', handleEvent)
    eventSource.addEventListener('THINKING_TEXT_MESSAGE_START', handleEvent)
    eventSource.addEventListener('THINKING_TEXT_MESSAGE_CONTENT', handleEvent)
    eventSource.addEventListener('THINKING_TEXT_MESSAGE_END', handleEvent)

    return eventSource
  }, [currentText])

  // Connect to SSE when session is created
  useEffect(() => {
    if (!agent || !session) return

    const eventSource = subscribeToEvents(agent.id, session.id)
    return () => eventSource.close()
  }, [agent, session, subscribeToEvents])

  // Send message
  const sendMessage = async () => {
    if (!input.trim() || !agent || !session || isLoading) return

    const userMessage = input.trim()
    setInput('')
    setMessages((prev) => [...prev, { role: 'user', content: userMessage }])
    setIsLoading(true)
    setError(null)

    try {
      const res = await fetch(
        `${API_BASE}/orgs/${DEFAULT_ORG}/agents/${agent.id}/sessions/${session.id}/messages`,
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            content: [{ type: 'text', text: userMessage }],
          }),
        }
      )
      if (!res.ok) {
        throw new Error('Failed to send message')
      }
    } catch (err) {
      setIsLoading(false)
      setError(err instanceof Error ? err.message : 'Failed to send')
    }
  }

  return (
    <div style={{ display: 'flex', height: '100vh' }}>
      {/* Chat Panel */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', padding: '20px' }}>
        <h1 style={{ marginBottom: '20px' }}>Everruns + CopilotKit Demo</h1>

        {error && (
          <div style={{
            padding: '10px',
            background: '#fee',
            color: '#c00',
            borderRadius: '4px',
            marginBottom: '10px'
          }}>
            {error}
          </div>
        )}

        {!agent || !session ? (
          <div>Initializing...</div>
        ) : (
          <>
            <div style={{
              flex: 1,
              overflowY: 'auto',
              border: '1px solid #ddd',
              borderRadius: '8px',
              padding: '10px',
              background: 'white',
              marginBottom: '10px'
            }}>
              {messages.map((msg, i) => (
                <div
                  key={i}
                  style={{
                    padding: '10px',
                    marginBottom: '10px',
                    borderRadius: '8px',
                    background: msg.role === 'user' ? '#e3f2fd' : '#f5f5f5',
                    marginLeft: msg.role === 'user' ? '40px' : '0',
                    marginRight: msg.role === 'assistant' ? '40px' : '0',
                  }}
                >
                  <strong>{msg.role === 'user' ? 'You' : 'Assistant'}</strong>
                  <p style={{ marginTop: '5px', whiteSpace: 'pre-wrap' }}>{msg.content}</p>
                </div>
              ))}
              {currentText && (
                <div
                  style={{
                    padding: '10px',
                    marginBottom: '10px',
                    borderRadius: '8px',
                    background: '#f5f5f5',
                    marginRight: '40px',
                  }}
                >
                  <strong>Assistant</strong>
                  <p style={{ marginTop: '5px', whiteSpace: 'pre-wrap' }}>{currentText}</p>
                </div>
              )}
              {isLoading && !currentText && (
                <div style={{ padding: '10px', color: '#666' }}>Thinking...</div>
              )}
            </div>

            <div style={{ display: 'flex', gap: '10px' }}>
              <input
                type="text"
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyPress={(e) => e.key === 'Enter' && sendMessage()}
                placeholder="Type a message..."
                disabled={isLoading}
                style={{
                  flex: 1,
                  padding: '10px',
                  borderRadius: '4px',
                  border: '1px solid #ddd',
                  fontSize: '16px',
                }}
              />
              <button
                onClick={sendMessage}
                disabled={isLoading || !input.trim()}
                style={{
                  padding: '10px 20px',
                  borderRadius: '4px',
                  border: 'none',
                  background: '#1976d2',
                  color: 'white',
                  cursor: isLoading ? 'not-allowed' : 'pointer',
                  opacity: isLoading ? 0.7 : 1,
                }}
              >
                Send
              </button>
            </div>
          </>
        )}
      </div>

      {/* Events Panel */}
      <div style={{
        width: '400px',
        borderLeft: '1px solid #ddd',
        padding: '20px',
        overflowY: 'auto',
        background: '#fafafa'
      }}>
        <h2 style={{ marginBottom: '10px' }}>AG-UI Events</h2>
        <p style={{ marginBottom: '10px', color: '#666', fontSize: '12px' }}>
          Agent: {agent?.id || 'loading...'}<br />
          Session: {session?.id || 'loading...'}
        </p>
        <div style={{
          fontFamily: 'monospace',
          fontSize: '11px',
          background: '#1e1e1e',
          color: '#d4d4d4',
          padding: '10px',
          borderRadius: '4px',
          overflowX: 'auto'
        }}>
          {events.length === 0 ? (
            <div style={{ color: '#666' }}>Waiting for events...</div>
          ) : (
            events.map((event, i) => (
              <div key={i} style={{ marginBottom: '5px' }}>
                <span style={{ color: '#569cd6' }}>{event.type}</span>
                {event.delta && <span style={{ color: '#ce9178' }}> "{event.delta.slice(0, 50)}..."</span>}
                {event.message && <span style={{ color: '#f44747' }}> {event.message}</span>}
                {event.toolCallName && <span style={{ color: '#4ec9b0' }}> {event.toolCallName}</span>}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  )
}

export default App
