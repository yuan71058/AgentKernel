
const { createApp, ref, computed, nextTick, onMounted } = Vue;

createApp({
  setup() {
    const wsUrl = ref('ws://localhost:9991/ws');
    const connected = ref(false);
    const connectionId = ref('');
    const sessionId = ref('debug_session');
    const chatInput = ref('');
    const chatMessages = ref([]);
    const sessions = ref([]);
    const selectedSessionId = ref('');
    const events = ref([]);
    const rawMessages = ref([]);
    const tools = ref([]);
    const rightTab = ref('summary');
    const statsExpanded = ref(false);
    const eventFilter = ref('all');
    const rawFilter = ref('all');
    const rawInput = ref('');
    const fullMessages = ref([]);
    const fullMessagesPage = ref(0);
    const fullMessagesTotal = ref(0);
    const fullMessagesHasMore = ref(true);
    const fullMessagesLoading = ref(false);
    const runtimeSessions = ref([]);
    const runtimeRuns = ref([]);
    const runtimeRuntimeStatus = ref('');
    const runtimeLoading = ref(false);
    const latestToolChainReport = ref(null);
    const latestTraceDetails = ref([]);

    const chatBody = ref(null);
    const streamBody = ref(null);
    const rawBody = ref(null);
    const fullMessagesBody = ref(null);

    const providerConfig = ref({
      protocol: 'openai',
      base_url: 'https://api.deepseek.com',
      api_key: '',
      model: 'deepseek-chat',
      max_tokens: 4096,
      temperature: 0,
    });
    const providerStatus = ref('');
    const systemPrompt = ref('');
    const systemPromptStatus = ref('');
    const contextPreview = ref({});
    const contextStatus = ref('');
    const contextKeepRecent = ref(20);
    const contextIncludeAfter = ref('');
    const contextSeedKind = ref('system_memory');
    const contextSeedContent = ref('');
    const contextSummary = ref('');

    const showPresetToolsModal = ref(false);
    const showApiKey = ref(false);

    const askUserQuestionState = ref(null);
    const pendingCommandWaiters = new Map();

    const AGENT_TOOL_PROVIDER_TEMPLATE = Object.freeze({
      protocol: 'openai',
      base_url: 'https://api.deepseek.com',
      api_key: 'sk-301169725af74465bb798b01de6fd150',
      model: 'deepseek-reasoner',
      max_tokens: 4096,
      temperature: 0.5,
    });
    const AGENT_TOOL_SYSTEM_PROMPT_TEMPLATE = '你是一个子agent，一次性对话的，不具备连续对话，所以你不能询问用户，如果真的要询问，必须让用户重新详细描述，而非“补充”';

    // 流式文本累积
    const streamingText = ref('');
    const streamingThinking = ref('');
    const isStreaming = ref(false);
    const currentRunId = ref('');
    const currentRunStatus = ref('idle');

    // 自动重连
    const autoReconnect = ref(true);
    const reconnectTimer = ref(null);

    let ws = null;
    let reqCounter = 0;
    // 记录发出的命令类型和所属 session（request_id → { command, sessionId }），用于响应回填隔离
    const pendingCommands = new Map();

    // ─── 预置工具定义 ──────────────────────────────────────
    const presetTools = [
      {
        name: 'get_time',
        description: '获取当前时间（北京时间 UTC+8）',
        schema: { type: 'object', properties: { format: { type: 'string', description: '输出格式: iso / locale / timestamp', default: 'locale' } } },
        tags: ['builtin', 'time'],
        execute(input) {
          const fmt = input.format || 'locale';
          const now = new Date();
          if (fmt === 'timestamp') return { ok: true, result: String(Date.now()) };
          if (fmt === 'iso') return { ok: true, result: now.toISOString() };
          return { ok: true, result: now.toLocaleString('zh-CN', { timeZone: 'Asia/Shanghai', hour12: false }) };
        }
      },
      {
        name: 'echo',
        description: '回显输入内容（调试用）',
        schema: { type: 'object', properties: { text: { type: 'string', description: '要回显的文本' } }, required: ['text'] },
        tags: ['builtin', 'debug'],
        execute(input) { return { ok: true, result: input.text || '' }; }
      },
      {
        name: 'calc',
        description: '简单数学计算（支持 + - * / % **）',
        schema: { type: 'object', properties: { expression: { type: 'string', description: '数学表达式，如 "2 + 3 * 4"' } }, required: ['expression'] },
        tags: ['builtin', 'math'],
        execute(input) {
          const expr = (input.expression || '').replace(/[^0-9+\-*/.%()\s]/g, '');
          if (!expr) return { ok: false, result: '无效表达式' };
          try {
            const val = Function('"use strict"; return (' + expr + ')')();
            return { ok: true, result: String(val) };
          } catch(e) { return { ok: false, result: '计算错误: ' + e.message }; }
        }
      },
      {
        name: 'random_number',
        description: '生成随机整数',
        schema: { type: 'object', properties: { min: { type: 'integer', default: 1 }, max: { type: 'integer', default: 100 } } },
        tags: ['builtin', 'random'],
        execute(input) {
          const min = Math.ceil(input.min ?? 1);
          const max = Math.floor(input.max ?? 100);
          const val = Math.floor(Math.random() * (max - min + 1)) + min;
          return { ok: true, result: String(val) };
        }
      },
      {
        name: 'generate_uuid',
        description: '生成 UUID v4',
        schema: { type: 'object', properties: {} },
        tags: ['builtin', 'util'],
        execute() {
          const uuid = ([1e7]+-1e3+-4e3+-8e3+-1e11).replace(/[018]/g, c =>
            (c ^ crypto.getRandomValues(new Uint8Array(1))[0] & 15 >> c / 4).toString(16));
          return { ok: true, result: uuid };
        }
      },
      {
        name: 'json_format',
        description: '格式化 JSON 字符串',
        schema: { type: 'object', properties: { json: { type: 'string', description: 'JSON 字符串' }, indent: { type: 'integer', default: 2 } }, required: ['json'] },
        tags: ['builtin', 'util'],
        execute(input) {
          try {
            const obj = JSON.parse(input.json);
            return { ok: true, result: JSON.stringify(obj, null, input.indent ?? 2) };
          } catch(e) { return { ok: false, result: 'JSON 解析错误: ' + e.message }; }
        }
      },
      {
        name: 'word_count',
        description: '统计文本字数/词数/行数',
        schema: { type: 'object', properties: { text: { type: 'string', description: '要统计的文本' } }, required: ['text'] },
        tags: ['builtin', 'text'],
        execute(input) {
          const text = input.text || '';
          const chars = text.length;
          const words = text.trim() ? text.trim().split(/\s+/).length : 0;
          const lines = text.split('\n').length;
          return { ok: true, result: `字符: ${chars}, 词: ${words}, 行: ${lines}` };
        }
      },
      {
        name: 'base64',
        description: 'Base64 编码/解码',
        schema: { type: 'object', properties: { text: { type: 'string' }, action: { type: 'string', enum: ['encode', 'decode'], default: 'encode' } }, required: ['text'] },
        tags: ['builtin', 'encoding'],
        execute(input) {
          try {
            if (input.action === 'decode') return { ok: true, result: atob(input.text) };
            return { ok: true, result: btoa(input.text) };
          } catch(e) { return { ok: false, result: e.message }; }
        }
      },
      {
        name: 'AskUserQuestion',
        description: '向用户提问并等待用户回答（阻塞交互）。用于需要用户决策或补充信息的场景。',
        schema: {
          type: 'object',
          properties: {
            question: { type: 'string', description: '要向用户提出的问题' },
            options: { type: 'array', items: { type: 'string' }, description: '可选的回答选项列表。若提供则显示为单选按钮，若不提供则显示文本输入框。' }
          },
          required: ['question']
        },
        tags: ['builtin', 'interactive'],
        execute(input) {
          return new Promise((resolve) => {
            askUserQuestionState.value = {
              question: input.question || '（未提供问题）',
              options: input.options || [],
              answer: '',
              resolve: (answer) => {
                askUserQuestionState.value = null;
                resolve({ ok: true, result: String(answer) });
              }
            };
          });
        }
      },
      {
        name: 'agent',
        description: '创建一个临时子会话，让 AI 调用 AI 完成一次独立对话，并把最终结果回传给当前工具调用。',
        schema: {
          type: 'object',
          properties: {
            message: { type: 'string', description: '要发给子 Agent 的消息内容' }
          },
          required: ['message']
        },
        tags: ['builtin', 'agent'],
        async execute(input) {
          const message = String(input?.message || '').trim();
          if (!message) {
            return { ok: false, result: 'agent 工具缺少 message 参数' };
          }
          return runNestedAgent(message);
        }
      },
    ];

    // 工具注册表（name → tool 定义）
    const toolRegistry = new Map();
    presetTools.forEach(t => toolRegistry.set(t.name, t));

    // Markdown 渲染函数
    function renderMarkdown(text) {
      if (!text) return '';
      try {
        const rawHtml = marked.parse(text);
        return DOMPurify.sanitize(rawHtml);
      } catch (e) {
        return text;
      }
    }

    /**
     * 注册本地工具处理器。
     * 业务端扩展时只需要关注 execute(input, ctx)，其他 call_id / WS 回包 / 日志由统一入口处理。
     */
    function defineClientTool(definition) {
      if (!definition || !definition.name || typeof definition.execute !== 'function') {
        throw new Error('defineClientTool requires { name, execute }');
      }
      const tool = {
        description: '',
        schema: { type: 'object', properties: {} },
        tags: [],
        ...definition,
      };
      toolRegistry.set(tool.name, tool);
      return tool;
    }

    function normalizeToolExecutionResult(value) {
      if (value && typeof value === 'object' && Object.prototype.hasOwnProperty.call(value, 'ok')) {
        return {
          ok: value.ok !== false,
          result: value.result == null ? '' : String(value.result),
        };
      }
      return { ok: true, result: value == null ? '' : String(value) };
    }

    // 工具执行日志
    const toolExecLog = ref([]);

    function registerPresetTool(t) {
      if (!ws || !t) return;
      const rid = nextReqId();
      const msg = {
        command: 'tool.register',
        request_id: rid,
        session_id: sessionId.value,
        payload: {
          tool_name: t.name,
          description: t.description,
          schema: t.schema,
          client_id: 'web_debug',
          timeout_ms: 0,
          tags: t.tags,
        }
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
      addLocalNotice('工具注册', `已发送注册请求: ${t.name}`);
    }

    function registerPresetTools() {
      if (!ws) return;
      presetTools.forEach(t => registerPresetTool(t));
    }

    function nextReqId() { return 'r' + (++reqCounter); }
    function now() { return new Date().toLocaleTimeString('zh-CN', { hour12: false }); }

    function runStatusLabel(status) {
      const map = {
        idle: '空闲',
        pending: '已发送',
        running: '运行中',
        streaming: '生成中',
        cancelling: '取消中',
        cancelled: '已中断',
        completed: '已完成',
        failed: '失败',
      };
      return map[status] || status || '未知';
    }

    function formatTimeoutLabel(timeoutMs) {
      if (timeoutMs == null) return '未设置';
      if (timeoutMs === 0) return '无限';
      return `${timeoutMs}ms`;
    }

    function formatToolChainIds(ids) {
      return Array.isArray(ids) && ids.length ? ids.join(', ') : '—';
    }

    function isAgentToolConfigured() {
      const provider = AGENT_TOOL_PROVIDER_TEMPLATE;
      const providerReady = [provider.base_url, provider.api_key, provider.model].every(v => typeof v === 'string' && !v.startsWith('__TODO_'));
      const promptReady = typeof AGENT_TOOL_SYSTEM_PROMPT_TEMPLATE === 'string' && !AGENT_TOOL_SYSTEM_PROMPT_TEMPLATE.startsWith('__TODO_');
      return providerReady && promptReady;
    }

    function createEphemeralSessionId(prefix = 'agent') {
      return `${prefix}_${Math.random().toString(36).slice(2, 10)}`;
    }

    function getAgentDelegatedTools() {
      return [...toolRegistry.values()].filter(tool => tool && tool.name && tool.name !== 'agent');
    }

    function toolChainIssueCount(report) {
      if (!report) return 0;
      return (report.dropped_incomplete_tool_call_ids?.length || 0) + (report.dropped_orphan_tool_result_ids?.length || 0);
    }

    function toolChainStatusText(report) {
      if (!report) return '暂无诊断';
      const issues = toolChainIssueCount(report);
      if (issues > 0) return `发现 ${issues} 个对齐问题`;
      if ((report.complete_tool_call_ids?.length || 0) > 0) return '工具链完整';
      return '本轮没有工具链问题';
    }

    function getPendingMeta(requestId) {
      const meta = pendingCommands.get(requestId);
      if (!meta) return { command: '', sessionId: '' };
      if (typeof meta === 'string') return { command: meta, sessionId: '' };
      return {
        command: meta.command || '',
        sessionId: meta.sessionId || '',
      };
    }

    function isCurrentSession(targetSessionId) {
      if (!targetSessionId) return true;
      return targetSessionId === sessionId.value;
    }

    function isTerminalRunStatus(status) {
      return ['idle', 'completed', 'cancelled', 'failed'].includes(status);
    }

    function appendAssistantMessage(text, meta = now()) {
      const normalized = String(text || '').trim();
      if (!normalized) return false;
      const lastMsg = chatMessages.value[chatMessages.value.length - 1];
      if (lastMsg && lastMsg.role === 'assistant' && String(lastMsg.text || '').trim() === normalized) {
        if (meta) lastMsg.meta = meta;
        return false;
      }
      chatMessages.value.push({ role: 'assistant', text: normalized, meta });
      return true;
    }

    function connect() {
      if (ws) ws.close();
      if (reconnectTimer.value) { clearTimeout(reconnectTimer.value); reconnectTimer.value = null; }
      try { ws = new WebSocket(wsUrl.value); } catch(e) { addLocalNotice('连接失败', e.message); scheduleReconnect(); return; }

      ws.onopen = () => {
        connected.value = true;
        addLocalNotice('WS', '连接建立');
        // 连接后只查询当前 session 状态，不自动注册工具
        loadSessions();
        loadSessionHistory();
        loadProvider();
        loadSystemPrompt();
        loadTools();
        loadContextPreview();
        loadRuntimeSessions();
      };
      ws.onclose = () => {
        pendingCommandWaiters.forEach(waiter => {
          waiter.reject(new Error('WS 连接已断开'));
        });
        pendingCommandWaiters.clear();
        connected.value = false;
        connectionId.value = '';
        currentRunId.value = '';
        currentRunStatus.value = 'idle';
        streamingText.value = '';
        streamingThinking.value = '';
        isStreaming.value = false;
        isStreaming.value = false;
        runtimeSessions.value = [];
        runtimeRuns.value = [];
        runtimeRuntimeStatus.value = '';
        runtimeLoading.value = false;
        latestToolChainReport.value = null;
        latestTraceDetails.value = [];
        addLocalNotice('WS', '连接断开');
        if (autoReconnect.value) scheduleReconnect();
      };
      ws.onerror = () => {
        addLocalNotice('WS', '连接错误');
      };
      ws.onmessage = (evt) => {
        const raw = evt.data;
        addRawMessage('in', raw);

        let msg;
        try { msg = JSON.parse(raw); } catch { return; }

        // Hello response
        if (msg.type === 'response' && msg.request_id === 'hello') {
          connectionId.value = msg.payload?.connection_id || '';
          return;
        }

        // Event
        if (msg.type === 'event') {
          const e = msg;
          const type = e.event_type || 'unknown';
          const payload = e.payload || {};
          addEvent(type, e.session_id, payload, e.run_id, e);
          handleEvent(type, payload, e);
          return;
        }

        // Response
        if (msg.type === 'response') {
          // 先刷入正在流式显示的文本（防止 model.completed 还没到时 response 先到导致丢失）
          flushStreaming();

          const pendingMeta = getPendingMeta(msg.request_id);
          const respCmd = pendingMeta.command;
          const respSessionId = msg.payload?.session_id || pendingMeta.sessionId || '';

          if (!msg.success) {
            addLocalNotice('命令失败', `[${msg.request_id}] ${msg.payload?.error || '未知错误'}`);
            if (respCmd === 'session.send' && isCurrentSession(respSessionId)) {
              currentRunStatus.value = 'failed';
              streamingText.value = '';
              streamingThinking.value = '';
              isStreaming.value = false;
              loadRuntimeSessions();
            }
          }

          // session.send 的响应不再 push 到聊天（assistant 消息已通过 model.completed 显示）
          // 其他 command 的响应照常显示为 system 消息

          // provider.get 响应 → 回填配置表单
          if (msg.success && msg.request_id.startsWith('r') && msg.payload?.provider && isCurrentSession(respSessionId)) {
            const p = msg.payload.provider;
            providerConfig.value.protocol = p.protocol || 'openai';
            providerConfig.value.base_url = p.base_url || '';
            providerConfig.value.api_key = p.api_key || '';
            providerConfig.value.model = p.model || '';
            providerConfig.value.max_tokens = p.max_tokens || 4096;
            providerConfig.value.temperature = p.temperature || 0;
            providerStatus.value = `已读取 (${msg.payload.is_override ? 'Session 覆盖' : '全局默认'})`;
            setTimeout(() => providerStatus.value = '', 4000);
          }
          // provider.update 响应
          if (msg.success && msg.payload?.provider && msg.request_id.startsWith('r') && isCurrentSession(respSessionId)) {
            const p = msg.payload.provider;
            providerStatus.value = `已保存: ${p.protocol} / ${p.model}`;
            setTimeout(() => providerStatus.value = '', 4000);
          }
          // system_prompt.get / set 响应
          if (msg.success && Object.prototype.hasOwnProperty.call(msg.payload || {}, 'system_prompt') && isCurrentSession(respSessionId)) {
            systemPrompt.value = msg.payload.system_prompt || '';
            systemPromptStatus.value = msg.payload.updated
              ? `已保存 (${msg.payload.is_session_override ? 'Session 覆盖' : '全局默认'})`
              : `已读取 (${msg.payload.is_session_override ? 'Session 覆盖' : '全局默认'})`;
            setTimeout(() => systemPromptStatus.value = '', 4000);
          }
          if (respCmd === 'session.messages.full') {
            fullMessagesLoading.value = false;
          }
          if (respCmd === 'runtime.sessions') {
            runtimeLoading.value = false;
          }
          if (msg.success && respCmd === 'run.cancel' && isCurrentSession(respSessionId)) {
            currentRunStatus.value = msg.payload?.status === 'cancelling' ? 'cancelling' : currentRunStatus.value;
            if (msg.payload?.run_id) currentRunId.value = msg.payload.run_id;
            loadRuntimeSessions();
            addLocalNotice(
              '推理',
              msg.payload?.cancelled
                ? `已请求中断 run: ${msg.payload.run_id || '未知'}`
                : `未找到可中断的 run: ${msg.payload?.run_id || '未知'}`
            );
          }
          if (msg.success && msg.payload?.active_context && isCurrentSession(respSessionId)) {
            contextPreview.value = {
              ...contextPreview.value,
              ...msg.payload,
              active_context: msg.payload.active_context,
            };
            const actionLabel = respCmd && respCmd.startsWith('context.')
              ? contextCommandName(respCmd)
              : '上下文已更新';
            contextStatus.value = `${actionLabel}：${contextModeLabel(msg.payload.active_context.mode)}`;
            setTimeout(() => contextStatus.value = '', 4000);
            if (respCmd && respCmd.startsWith('context.') && respCmd !== 'context.preview') {
              loadContextPreview();
            }
          }
          if (msg.success && msg.payload?.seed && respCmd === 'context.seed.add' && isCurrentSession(respSessionId)) {
            contextSeedContent.value = '';
            loadContextPreview();
          }
          if (msg.success && (respCmd === 'context.compaction.apply') && isCurrentSession(respSessionId)) {
            contextSummary.value = '';
            loadContextPreview();
          }

          // tool.list 响应 → 回填工具列表
          if (msg.success && Array.isArray(msg.payload?.tools) && isCurrentSession(respSessionId)) {
            tools.value = msg.payload.tools.map(t => ({
              name: t.name,
              description: t.description,
              client: t.client_id || '—',
              timeout: formatTimeoutLabel(t.timeout_ms),
              tags: t.tags || [],
              input_schema: t.input_schema || {},
            }));
          }

          if (msg.success && respCmd === 'runtime.sessions') {
            runtimeSessions.value = Array.isArray(msg.payload?.sessions) ? msg.payload.sessions : [];
            runtimeRuns.value = Array.isArray(msg.payload?.runs) ? msg.payload.runs : [];
            runtimeRuntimeStatus.value = `已刷新 ${now()}`;

            const activeTrackedRun = currentRunId.value
              ? runtimeRuns.value.find(r => r.run_id === currentRunId.value)
              : null;
            const fallbackSessionRun = !currentRunId.value
              ? runtimeRuns.value.find(r => r.session_id === sessionId.value)
              : null;

            if (activeTrackedRun) {
              if (activeTrackedRun.status === 'cancelling') {
                currentRunStatus.value = 'cancelling';
              } else if (isTerminalRunStatus(currentRunStatus.value)) {
                currentRunStatus.value = activeTrackedRun.status;
              }
            } else if (fallbackSessionRun) {
              currentRunId.value = fallbackSessionRun.run_id;
              currentRunStatus.value = fallbackSessionRun.status;
            } else {
              if (['pending', 'running', 'streaming', 'cancelling'].includes(currentRunStatus.value)) {
                currentRunStatus.value = 'idle';
                currentRunId.value = '';
              }
            }
          }

          // tool.register / unregister 响应后刷新 session 工具配置
          if (msg.success && (respCmd === 'tool.register' || respCmd === 'tool.unregister')) {
            loadTools();
          }

          // session.list 响应 → 回填 session 下拉
          if (msg.success && respCmd === 'session.list' && Array.isArray(msg.payload?.sessions)) {
            sessions.value = msg.payload.sessions.filter(item => item && typeof item === 'object' && item.session_id);
            selectedSessionId.value = sessionId.value;
          }
          if (msg.success && respCmd === 'session.delete') {
            const deletedSessionId = msg.payload?.session_id || respSessionId;
            const remainingSessions = sessions.value.filter(item => item.session_id !== deletedSessionId);
            sessions.value = remainingSessions;
            selectedSessionId.value = deletedSessionId === sessionId.value
              ? (remainingSessions[0]?.session_id || '')
              : sessionId.value;
            if (deletedSessionId === sessionId.value) {
              if (remainingSessions.length > 0) {
                selectSession(remainingSessions[0].session_id);
                addLocalNotice('会话', `已删除 session: ${deletedSessionId}，已切换到 ${remainingSessions[0].session_id}`);
              } else {
                const newId = 'session_' + Math.random().toString(36).substring(2, 8);
                sessionId.value = newId;
                selectedSessionId.value = newId;
                resetSessionViewState();
                sessions.value = [{ session_id: newId, title: '新会话', message_count: 0 }];
                addLocalNotice('会话', `已删除 session: ${deletedSessionId}，已切换到新会话: ${newId}`);
              }
            } else {
              addLocalNotice('会话', `已删除 session: ${deletedSessionId}`);
            }
            loadSessions();
          }

          // session.send 响应 → 如果 model.completed 还没来，用 response payload 兜底显示
          if (msg.success && respCmd === 'session.send' && msg.payload?.run_id && isCurrentSession(respSessionId)) {
            currentRunId.value = msg.payload.run_id;
            currentRunStatus.value = msg.payload.status || currentRunStatus.value;
          }
          if (Array.isArray(msg.payload?.trace_details) && isCurrentSession(respSessionId)) {
            latestTraceDetails.value = msg.payload.trace_details;
            const latestTrace = msg.payload.trace_details[msg.payload.trace_details.length - 1];
            if (latestTrace?.tool_chain_report) {
              latestToolChainReport.value = latestTrace.tool_chain_report;
            }
          }
          if (msg.success && msg.payload?.status === 'cancelled' && isCurrentSession(respSessionId)) {
            currentRunStatus.value = 'cancelled';
            if (msg.payload?.content) {
              appendAssistantMessage(
                msg.payload.content,
                msg.payload.partial_preserved ? '已中断，已保留中断前输出' : '已中断'
              );
            }
            streamingText.value = '';
            streamingThinking.value = '';
            isStreaming.value = false;
            scrollToChat();
          } else if (msg.success && msg.payload?.content && typeof msg.payload.content === 'string' && isCurrentSession(respSessionId)) {
            // 只有当最后一条不是 assistant 消息时才兜底推送（避免和 model.completed 或 flushStreaming 重复）
            const lastMsg = chatMessages.value[chatMessages.value.length - 1];
            if (!lastMsg || lastMsg.role !== 'assistant') {
              chatMessages.value.push({ role: 'assistant', text: msg.payload.content, meta: now() });
              scrollToChat();
            }
          }

          // session.messages 响应 → 根据请求来源回填聊天历史或全量消息面板
          if (msg.success && msg.payload?.messages && Array.isArray(msg.payload.messages) && isCurrentSession(respSessionId)) {
            const msgs = msg.payload.messages;
            if (respCmd === 'session.messages.full') {
              fullMessagesTotal.value = msg.payload.total || 0;
              fullMessagesHasMore.value = (msg.payload.page + 1) < (msg.payload.pages || 0);
              const mapped = msgs.map(m => ({ ...m, _new: true, _expanded: false }));
              if ((msg.payload.page || 0) === 0) fullMessages.value = mapped;
              else fullMessages.value.push(...mapped);
              fullMessagesPage.value = (msg.payload.page || 0) + 1;
              fullMessagesLoading.value = false;
              setTimeout(() => fullMessages.value.forEach(m => m._new = false), 800);
            } else if (msgs.length > 0) {
              const mappedMessages = msgs.map(m => ({
                role: m.role === 'assistant' ? 'assistant' : m.role === 'user' ? 'user' : 'system',
                text: extractHistoryMessageText(m),
                meta: m.created_at ? new Date(m.created_at).toLocaleTimeString('zh-CN', {hour12:false}) : '',
              })).filter(m => m.text);
              chatMessages.value = mappedMessages;
              scrollToChat();
              const loadedCount = msgs.length;
              const totalCount = msg.payload.total || msgs.length;
              const renderedCount = chatMessages.value.length;
              addLocalNotice(
                '会话',
                renderedCount === msgs.length
                  ? (totalCount > loadedCount
                      ? `已加载 ${loadedCount} 条历史消息（会话共 ${totalCount} 条）`
                      : `已加载 ${loadedCount} 条历史消息`)
                  : `已加载 ${loadedCount} 条历史消息，当前显示 ${renderedCount} 条`
              );
            }
          }
          const pendingWaiter = pendingCommandWaiters.get(msg.request_id);
          if (pendingWaiter) {
            pendingCommandWaiters.delete(msg.request_id);
            if (msg.success) {
              pendingWaiter.resolve(msg);
            } else {
              pendingWaiter.reject(new Error(msg.payload?.error || '未知错误'));
            }
          }
          pendingCommands.delete(msg.request_id);
          return;
        }

        // Stream 是 Core 主动推送的运行时流，也归入事件流；命令响应不进入事件流
        if (msg.type === 'stream') {
          addEvent('stream', msg.session_id, msg.data || {}, msg.run_id, msg);
          return;
        }
      };
    }

    function disconnect() {
      autoReconnect.value = false;
      if (reconnectTimer.value) { clearTimeout(reconnectTimer.value); reconnectTimer.value = null; }
      if (ws) ws.close();
    }

    function scheduleReconnect() {
      if (reconnectTimer.value) return;
      reconnectTimer.value = setTimeout(() => {
        reconnectTimer.value = null;
        if (!connected.value && autoReconnect.value) {
          addLocalNotice('WS', '自动重连...');
          connect();
        }
      }, 3000);
    }

    function loadSessionHistory() {
      if (!ws || !sessionId.value) return;
      // 1. 拉取消息历史
      const rid = nextReqId();
      const fullMsg = {
        command: 'session.messages',
        request_id: rid,
        session_id: sessionId.value,
        payload: { page: 0, limit: 50 }
      };
      const out = JSON.stringify(fullMsg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function loadSessions() {
      if (!ws) return;
      const rid = nextReqId();
      const msg = {
        command: 'session.list',
        request_id: rid,
        session_id: '',
        payload: { page: 0, limit: 100 },
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function resetSessionViewState() {
      chatMessages.value = [];
      streamingText.value = '';
      isStreaming.value = false;
      currentRunId.value = '';
      currentRunStatus.value = 'idle';
      events.value = [];
      rawMessages.value = [];
      toolExecLog.value = [];
      fullMessages.value = [];
      fullMessagesPage.value = 0;
      fullMessagesTotal.value = 0;
      fullMessagesHasMore.value = true;
      fullMessagesLoading.value = false;
      runtimeSessions.value = [];
      runtimeRuns.value = [];
      runtimeRuntimeStatus.value = '';
      runtimeLoading.value = false;
      latestToolChainReport.value = null;
      latestTraceDetails.value = [];
    }

    function createNewSession() {
      const newId = 'session_' + Math.random().toString(36).substring(2, 8);
      sessions.value.unshift({ session_id: newId, title: '新会话', message_count: 0 });
      selectSession(newId);
    }

    function deleteSession(id) {
      if (!ws || !id) return;
      if (runtimeSessions.value.includes(id)) {
        addLocalNotice('会话', `session ${id} 当前仍有运行中的任务，暂不允许删除`);
        return;
      }
      const target = sessions.value.find(s => s.session_id === id);
      const title = target?.title || id;
      if (!window.confirm(`确认删除 session "${title}" 吗？\n\n注意：这会移除该 session 索引与当前上下文视图。`)) {
        return;
      }
      sendCommand('session.delete', {}, id);
    }

    function selectSession(id) {
      if (!id || id === sessionId.value) return;
      sessionId.value = id;
      resetSessionViewState();
      loadSessionHistory();
      loadProvider();
      loadSystemPrompt();
      loadTools();
      loadContextPreview();
      loadRuntimeSessions();
      addLocalNotice('会话', `已切换并加载 session: ${id}`);
    }

    function sendCommand(command, payload = {}, sid = sessionId.value) {
      if (!ws) return '';
      const rid = nextReqId();
      const msg = { command, request_id: rid, session_id: sid, payload };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
      return rid;
    }

    function sendCommandAwait(command, payload = {}, sid = sessionId.value, options = {}) {
      if (!ws) {
        return Promise.reject(new Error('WS 未连接，无法执行命令'));
      }
      const timeoutMs = options.timeoutMs ?? 120000;
      const rid = sendCommand(command, payload, sid);
      return new Promise((resolve, reject) => {
        const timer = timeoutMs > 0 ? setTimeout(() => {
          pendingCommandWaiters.delete(rid);
          pendingCommands.delete(rid);
          reject(new Error(`${command} 等待响应超时 (${timeoutMs}ms)`));
        }, timeoutMs) : null;
        pendingCommandWaiters.set(rid, {
          resolve: (msg) => {
            if (timer) clearTimeout(timer);
            resolve(msg);
          },
          reject: (err) => {
            if (timer) clearTimeout(timer);
            reject(err instanceof Error ? err : new Error(String(err || '未知错误')));
          },
        });
      });
    }

    async function registerToolForSession(tool, sid) {
      if (!tool || !sid) return;
      await sendCommandAwait('tool.register', {
        tool_name: tool.name,
        description: tool.description || '',
        schema: tool.schema || { type: 'object', properties: {} },
        client_id: 'web_debug_agent',
        timeout_ms: 0,
        tags: tool.tags || [],
      }, sid);
    }

    async function runNestedAgent(message) {
      if (!isAgentToolConfigured()) {
        return {
          ok: false,
          result: 'agent 工具尚未配置完成，请先在 script.js 中补齐 AGENT_TOOL_PROVIDER_TEMPLATE 与 AGENT_TOOL_SYSTEM_PROMPT_TEMPLATE 占位符。',
        };
      }

      const agentSessionId = createEphemeralSessionId();
      try {
        // system_prompt.set 会确保 session 先被创建，随后再写 provider 覆盖。
        await sendCommandAwait('system_prompt.set', {
          system_prompt: AGENT_TOOL_SYSTEM_PROMPT_TEMPLATE,
        }, agentSessionId);

        await sendCommandAwait('provider.update', {
          ...AGENT_TOOL_PROVIDER_TEMPLATE,
        }, agentSessionId);

        for (const tool of getAgentDelegatedTools()) {
          await registerToolForSession(tool, agentSessionId);
        }

        const response = await sendCommandAwait('session.send', {
          message,
          max_repeated_tool_calls: 10,
        }, agentSessionId, { timeoutMs: 0 });
        const content = response?.payload?.content;
        return {
          ok: true,
          result: typeof content === 'string' && content.trim()
            ? content
            : JSON.stringify(response?.payload || {}, null, 2),
        };
      } catch (error) {
        return {
          ok: false,
          result: `agent 子会话执行失败: ${error?.message || error}`,
        };
      } finally {
        try {
          await sendCommandAwait('session.delete', {}, agentSessionId, { timeoutMs: 10000 });
        } catch (cleanupError) {
          addLocalNotice('agent', `子会话清理失败: ${cleanupError?.message || cleanupError}`);
        }
        loadSessions();
      }
    }

    function loadFullMessages(reset = false) {
      if (!ws || !sessionId.value || fullMessagesLoading.value) return;
      if (reset) {
        fullMessages.value = [];
        fullMessagesPage.value = 0;
        fullMessagesTotal.value = 0;
        fullMessagesHasMore.value = true;
      }
      if (!fullMessagesHasMore.value && !reset) return;
      fullMessagesLoading.value = true;
      const rid = nextReqId();
      const msg = {
        command: 'session.messages',
        request_id: rid,
        session_id: sessionId.value,
        payload: { page: fullMessagesPage.value, limit: 200 }
      };
      pendingCommands.set(rid, 'session.messages.full');
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function onFullMessagesScroll() {
      const el = fullMessagesBody.value;
      if (!el || fullMessagesLoading.value || !fullMessagesHasMore.value) return;
      if (el.scrollTop + el.clientHeight >= el.scrollHeight - 80) {
        loadFullMessages(false);
      }
    }

    function roleLabel(role) {
      return ({ user: '用户', assistant: '助手', system: '系统', tool: '工具' })[role] || role || '未知';
    }

    function roleColor(role) {
      return ({ user: 'var(--green)', assistant: 'var(--cyan)', system: 'var(--orange)', tool: 'var(--pink)' })[role] || 'var(--text2)';
    }

    function formatMessageTime(value) {
      if (!value) return '';
      try { return new Date(value).toLocaleTimeString('zh-CN', {hour12:false}); } catch { return value; }
    }

    function extractHistoryMessageText(message) {
      if (!message) return '';
      if (typeof message.text === 'string' && message.text) return message.text;
      if (Array.isArray(message.content)) {
        return message.content.map(block => {
          if (!block || typeof block !== 'object') return '';
          if (block.type === 'text') return block.text || '';
          if (block.type === 'tool_result') return block.content || '';
          return '';
        }).join('').trim();
      }
      return '';
    }

    function contextCommandName(cmd) {
      const map = {
        'context.preview': '上下文预览',
        'context.reset': '重置上下文',
        'context.exclude': '排除上下文消息',
        'context.include_after': '从指定消息后开始',
        'context.keep_recent': '仅保留最近 N 条',
        'context.seed.add': '注入上下文块',
        'context.compaction.apply': '应用压缩摘要',
      };
      return map[cmd] || cmd;
    }

    function contextModeLabel(mode) {
      const map = { full: '全量上下文', sliding: '滑动/裁剪上下文', compacted: '压缩上下文' };
      return map[mode] || mode || '未知';
    }

    function seedKindLabel(kind) {
      const map = { system_memory: '系统记忆', compaction_summary: '压缩摘要', user_preference: '用户偏好', world_state: '世界状态', agent_state: 'Agent状态' };
      return map[kind] || kind || '未知';
    }

    function loadContextPreview() {
      sendCommand('context.preview');
    }

    function resetContext() {
      sendCommand('context.reset');
    }

    function keepRecentContext() {
      const keep = Number(contextKeepRecent.value || 0);
      sendCommand('context.keep_recent', { keep: keep > 0 ? keep : null });
    }

    function includeAfterContext() {
      const message_id = contextIncludeAfter.value.trim();
      if (!message_id) return;
      sendCommand('context.include_after', { message_id });
    }

    function excludeSingleMessage(messageId) {
      sendCommand('context.exclude', { start_message_id: messageId, end_message_id: messageId });
    }

    function addContextSeed() {
      sendCommand('context.seed.add', {
        kind: contextSeedKind.value,
        content: contextSeedContent.value,
        enabled: true,
        priority: 0,
      });
    }

    function applyCompaction() {
      sendCommand('context.compaction.apply', {
        summary: contextSummary.value,
        include_after_message_id: contextIncludeAfter.value.trim() || null,
      });
    }

    function contextMessageText(m) {
      return (m.content || []).map(c => {
        if (c.type === 'text') return c.text || '';
        if (c.type === 'tool_use') return `[tool_use:${c.name}] ${JSON.stringify(c.input || {})}`;
        if (c.type === 'tool_result') return `[tool_result:${c.tool_use_id}] ${c.content || ''}`;
        return JSON.stringify(c);
      }).join('\n');
    }

    function loadTools() {
      if (!ws) return;
      const rid = nextReqId();
      const msg = {
        command: 'tool.list',
        request_id: rid,
        session_id: sessionId.value,
        payload: {},
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function loadRuntimeSessions() {
      if (!ws) return;
      runtimeLoading.value = true;
      sendCommand('runtime.sessions', {}, '');
    }

    function loadSystemPrompt() {
      if (!ws) return;
      const rid = nextReqId();
      const msg = {
        command: 'system_prompt.get',
        request_id: rid,
        session_id: sessionId.value,
        payload: {},
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function saveSystemPrompt() {
      if (!ws) return;
      const rid = nextReqId();
      const msg = {
        command: 'system_prompt.set',
        request_id: rid,
        session_id: sessionId.value,
        payload: { system_prompt: systemPrompt.value || '' },
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
      systemPromptStatus.value = '已发送保存请求...';
      setTimeout(() => systemPromptStatus.value = '', 3000);
    }

    function sendChat() {
      if (!ws || !chatInput.value.trim()) return;
      const text = chatInput.value.trim();
      chatMessages.value.push({ role: 'user', text, meta: now() });
      chatInput.value = '';
      currentRunId.value = '';
      currentRunStatus.value = 'pending';
      streamingText.value = '';
      streamingThinking.value = '';
      isStreaming.value = false;
      scrollToChat();

      const rid = nextReqId();
      const fullMsg = {
        command: 'session.send',
        request_id: rid,
        session_id: sessionId.value,
        payload: { message: text }
      };
      const out = JSON.stringify(fullMsg);
      ws.send(out);
      addRawMessage('out', out);
      loadRuntimeSessions();
    }

    function cancelCurrentRun() {
      if (!ws || !currentRunId.value) return;
      currentRunStatus.value = 'cancelling';
      sendCommand('run.cancel', { run_id: currentRunId.value });
    }

    function sendRaw() {
      if (!ws || !rawInput.value.trim()) return;
      const text = rawInput.value.trim();
      let parsed;
      try { parsed = JSON.parse(text); } catch(e) { addLocalNotice('原始命令', 'JSON 解析失败: ' + e.message); return; }
      if (!parsed.request_id) parsed.request_id = nextReqId();
      const out = JSON.stringify(parsed);
      ws.send(out);
      addRawMessage('out', out);
      rawInput.value = '';
    }

    function sendToolResult(call_id, result, is_error, sid = sessionId.value) {
      if (!ws) return;
      const msg = {
        command: 'tool.execute.result',
        request_id: nextReqId(),
        session_id: sid,
        payload: { call_id, result: String(result), is_error: !!is_error },
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function flushStreaming() {
      // 将正在流式显示的内容刷入正式消息列表（避免 response 和 event 竞争丢失）
      if (isStreaming.value && streamingText.value) {
        appendAssistantMessage(streamingText.value, now());
        streamingText.value = '';
        isStreaming.value = false;
        scrollToChat();
      }
    }

    // 工具队列管理，用于确保带有阻塞的工具按顺序执行
    const toolCallQueue = [];
    let isProcessingToolCall = false;

    async function processNextToolCall() {
      if (isProcessingToolCall || toolCallQueue.length === 0) return;
      isProcessingToolCall = true;
      const { payload, eventSessionId } = toolCallQueue.shift();
      try {
        await executeToolCall(payload, eventSessionId);
      } finally {
        isProcessingToolCall = false;
        processNextToolCall();
      }
    }

    async function executeToolCall(payload, eventSessionId) {
      const { tool_name, call_id, input } = payload || {};
      const safeInput = input || {};
      const tool = toolRegistry.get(tool_name);
      const startTime = Date.now();
      const belongsToCurrentSession = isCurrentSession(eventSessionId);

      // 先结束流式，避免工具执行日志插入时和模型增量混在一起
      if (belongsToCurrentSession) {
        flushStreaming();
      }

      let execResult;
      if (!tool) {
        execResult = { ok: false, result: `tool '${tool_name}' not found in client registry` };
      } else {
        try {
          const value = await tool.execute(safeInput, {
            toolName: tool_name,
            callId: call_id,
            sessionId: eventSessionId || sessionId.value,
            connectionId: connectionId.value,
          });
          execResult = normalizeToolExecutionResult(value);
        } catch(e) {
          execResult = { ok: false, result: '执行异常: ' + e.message };
        }
      }

      const elapsed = Date.now() - startTime;
      if (belongsToCurrentSession) {
        chatMessages.value.push({
          role: 'system',
          text: tool
            ? `🔧 ${tool_name}(${JSON.stringify(safeInput)})\n→ ${execResult.result} [${elapsed}ms]`
            : `⚠️ 未知工具: ${tool_name}\n输入: ${JSON.stringify(safeInput)}`,
          meta: `call_id: ${call_id}`
        });

        toolExecLog.value.push({
          time: now(), tool: tool_name, input: JSON.stringify(safeInput),
          result: execResult.result, ok: execResult.ok, elapsed,
        });
        if (toolExecLog.value.length > 100) toolExecLog.value.splice(0, toolExecLog.value.length - 100);
        scrollToChat();
      }

      sendToolResult(call_id, execResult.result, !execResult.ok, eventSessionId || sessionId.value);
    }

    async function handleToolCallRequest(payload, eventSessionId) {
      if (payload?.tool_name === 'agent') {
        void executeToolCall(payload, eventSessionId);
        return;
      }
      toolCallQueue.push({ payload, eventSessionId });
      processNextToolCall();
    }

    function handleEvent(type, payload, envelope = {}) {
      const eventSessionId = envelope.session_id || payload?.session_id || '';
      const eventRunId = envelope.run_id || payload?.run_id || '';
      const belongsToCurrentSession = isCurrentSession(eventSessionId);
      if (belongsToCurrentSession && type === 'run.started' && eventRunId && (!currentRunId.value || currentRunStatus.value === 'pending')) {
        currentRunId.value = eventRunId;
      }
      const belongsToTrackedRun = !eventRunId || !currentRunId.value || eventRunId === currentRunId.value;
      // tool.call.request → 统一工具调用入口；业务端只需要注册工具 execute(input, ctx)
      if (type === 'tool.call.request') {
        handleToolCallRequest(payload, eventSessionId);
        return;
      }
      if (!belongsToCurrentSession) {
        if (type === 'run.started' || type === 'run.completed' || type === 'run.cancelled' || type === 'error') {
          loadRuntimeSessions();
        }
        return;
      }
      if (type === 'run.started') {
        if (!belongsToTrackedRun) {
          loadRuntimeSessions();
          return;
        }
        currentRunStatus.value = 'running';
        streamingThinking.value = '';
        loadRuntimeSessions();
        return;
      }
      if (type === 'tool_chain.diagnosed') {
        latestToolChainReport.value = payload.report || null;
        return;
      }
      // model.delta → 实时累积流式文本
      if (type === 'model.delta' && payload.delta) {
        if (!belongsToTrackedRun) return;
        if (payload.event_type === 'thinking') {
          streamingThinking.value += payload.delta;
        } else {
          currentRunStatus.value = 'streaming';
          isStreaming.value = true;
          streamingText.value += payload.delta;
        }
        scrollToChat();
      }
      // model.completed → 最终确认（不用 setTimeout，避免与 response 到达产生竞争）
      if (type === 'model.completed' && payload.content) {
        if (!belongsToTrackedRun) return;
        // 用最终 content 确保完整
        if (payload.content) {
          streamingText.value = payload.content;
        }
        isStreaming.value = false;
        // 立即转为正式消息
        appendAssistantMessage(streamingText.value || payload.content, now());
        streamingText.value = '';
        streamingThinking.value = '';
        currentRunStatus.value = 'completed';
        scrollToChat();
        return;
      }
      if (type === 'run.completed') {
        if (!belongsToTrackedRun) {
          loadRuntimeSessions();
          return;
        }
        currentRunStatus.value = 'completed';
        loadRuntimeSessions();
        scrollToChat();
        return;
      }
      if (type === 'run.cancelled') {
        if (!belongsToTrackedRun) {
          loadRuntimeSessions();
          return;
        }
        const partialContent = payload.partial_content || streamingText.value || '';
        if (partialContent) {
          appendAssistantMessage(
            partialContent,
            payload.preserved ? '已中断，已保留中断前输出' : '已中断'
          );
        }
        streamingText.value = '';
        streamingThinking.value = '';
        isStreaming.value = false;
        currentRunStatus.value = 'cancelled';
        loadRuntimeSessions();
        scrollToChat();
        return;
      }
      if (type === 'error') {
        if (!belongsToTrackedRun) {
          loadRuntimeSessions();
          return;
        }
        currentRunStatus.value = 'failed';
        loadRuntimeSessions();
      }
      // tool.registered → 更新工具列表
      if (type === 'tool.registered') {
        // tools will be refreshed from tool.register commands
      }
      // model.delta → 可以选择实时显示流式内容
    }

    // ─── 事件类型中文标签 ────────────────────────────────
    function eventLabel(type) {
      const map = {
        'model.delta': '模型流式增量',
        'model.completed': '模型完成',
        'tool_chain.diagnosed': '工具链诊断',
        'session.created': '会话创建',
        'session.closed': '会话关闭',
        'run.started': '推理开始',
        'run.cancelled': '推理中断',
        'run.completed': '推理完成',
        'tool.call.request': '工具执行请求',
        'tool.call.result': 'Core 已确认工具结果',
        'tool.call.error': '工具调用失败',
        'tool.registered': '工具注册',
        'context.threshold.reached': '上下文阈值',
        'context.updated': '上下文更新',
        'context.reset': '上下文重置',
        'context.seed.added': 'Seed 已注入',
        'context.compaction.applied': '上下文压缩',
        'prompt.attached': 'Prompt 附加',
        'error': '错误',
        'stream': '流数据',
      };
      return map[type] || type;
    }

    function summarizeEvent(type, p) {
      p = p || {};
      const text = (v, n = 60) => String(v ?? '').slice(0, n);
      const tag = (v, cls) => `<span class="raw-tag${cls ? ' '+cls : ''}">${String(v ?? '')}</span>`;
      switch(type) {
        case 'model.delta': return `${p.event_type === 'thinking' ? '思维' : '文本'}增量 ${tag(JSON.stringify(p.delta || '').slice(0, 48))}`;
        case 'model.completed': return `内容 ${tag(text(p.content, 80))}`;
        case 'tool_chain.diagnosed':
          return `${tag(toolChainStatusText(p.report))} 完整 ${tag((p.report?.complete_tool_call_ids || []).length)} 丢弃 ${tag(toolChainIssueCount(p.report))}`;
        case 'tool.call.request': return `${tag(p.tool_name)} 输入 ${tag(JSON.stringify(p.input || {}).slice(0, 64))}`;
        case 'tool.call.result': return `${tag(p.tool_name)} ${p.is_error ? tag('失败','err') : tag('成功')} 结果 ${text(p.result, 56)}`;
        case 'tool.call.error': return `${tag(p.tool_name)} ${tag(text(p.error, 64), 'err')}`;
        case 'tool.registered': return `${tag(p.tool_name)} 客户端 ${tag(p.client_id || '—')}`;
        case 'session.created': return `标题 ${tag(p.title || (p.auto_created ? '自动创建' : '—'))}`;
        case 'session.closed': return p.reason ? `原因 ${tag(p.reason)}` : '会话关闭';
        case 'run.started': return `供应商 ${tag(p.provider || '—')} 模型 ${tag(p.model || '—')}`;
        case 'run.cancelled': return `${p.preserved ? tag('已保留部分输出') : tag('未保留输出')} 耗时 ${tag(p.duration_ms ? p.duration_ms + 'ms' : '—')}`;
        case 'run.completed': return `Token ${tag(p.total_tokens || '—')} 耗时 ${tag(p.duration_ms ? p.duration_ms + 'ms' : '—')}`;
        case 'error': return tag(JSON.stringify(p).slice(0, 90), 'err');
        case 'context.threshold.reached': return `使用率 ${tag((p.usage_percent ?? '—') + '%')} Token ${tag(p.estimated_tokens || '—')}`;
        case 'context.updated': return `动作 ${tag(contextCommandName(p.action || 'context.updated'))} Mode ${tag(contextModeLabel(p.context?.mode || '—'))}`;
        case 'context.reset': return `Mode ${tag(p.context?.mode || 'full')}`;
        case 'context.seed.added': return `Seed ${tag(p.seed?.seed_id || '—')} ${tag(p.seed?.kind || '')}`;
        case 'context.compaction.applied': return `压缩已应用 ${tag(p.context?.context_id || '')}`;
        case 'prompt.attached': return `Prompt ${tag(p.prompt_name || p.name || '—')}`;
        case 'stream': return `流数据 ${tag(JSON.stringify(p).slice(0, 80))}`;
        default: return JSON.stringify(p).slice(0, 100);
      }
    }

    function eventCategory(type) {
      if (type === 'error') return 'error';
      if (type.startsWith('model.')) return 'model';
      if (type.startsWith('run.')) return 'run';
      if (type.startsWith('tool_chain.')) return 'tool';
      if (type.startsWith('tool.')) return 'tool';
      if (type.startsWith('session.')) return 'session';
      if (type.startsWith('context.')) return 'context';
      return 'other';
    }

    function addEvent(type, session, payload, runId, rawEnvelope) {
      const data = JSON.stringify(rawEnvelope || { type: 'event', event_type: type, session_id: session, run_id: runId, payload }, null, 2);
      const category = eventCategory(type);
      const shouldMerge = type === 'model.delta';
      const last = events.value[events.value.length - 1];
      if (
        shouldMerge &&
        last &&
        last._merged &&
        last.type === type &&
        last.runId === runId &&
        last.session === session &&
        last._mergeKey === `${type}:${payload?.event_type || 'text'}`
      ) {
        last.count++;
        last.children.push({
          time: now(),
          payload,
          data,
          brief: summarizeEvent(type, payload),
        });
        last.data = data;
        last.time = now();
        last.brief = `${payload?.event_type === 'thinking' ? '思维' : '文本'}增量，已聚合 ${last.count} 条`;
        nextTick(() => { if (streamBody.value) streamBody.value.scrollTop = streamBody.value.scrollHeight; });
        return;
      }

      events.value.push({
        type,
        category,
        session,
        brief: summarizeEvent(type, payload),
        runId,
        data,
        time: now(),
        label: eventLabel(type),
        _new: true,
        _expanded: false,
        _merged: shouldMerge,
        _mergeKey: `${type}:${payload?.event_type || 'text'}`,
        count: 1,
        children: shouldMerge ? [{
          time: now(),
          payload,
          data,
          brief: summarizeEvent(type, payload),
        }] : [],
      });
      if (events.value.length > 500) events.value.splice(0, events.value.length - 500);
      nextTick(() => { if (streamBody.value) streamBody.value.scrollTop = streamBody.value.scrollHeight; });
      setTimeout(() => { const e = events.value[events.value.length-1]; if(e) e._new = false; }, 800);
    }

    function addLocalNotice(title, message) {
      chatMessages.value.push({ role: 'system', text: `${title}: ${message || ''}`, meta: now() });
      scrollToChat();
    }

    // ─── 原始消息中文解析 ──────────────────────────────
    function summarizeRaw(dir, raw) {
      let msg;
      try { msg = JSON.parse(raw); } catch { return { label: '无法解析', brief: raw.slice(0, 60) }; }

      const tag = (v, cls) => `<span class="raw-tag${cls ? ' '+cls : ''}">${v}</span>`;

      // ─── 收到的消息 ───
      if (dir === 'in') {
        // Response
        if (msg.type === 'response') {
          if (msg.request_id === 'hello') {
            return { label: '连接确认', brief: `连接ID ${tag((msg.payload?.connection_id||'').slice(0,8))}` };
          }
          const status = msg.success ? '成功' : '失败';
          const cls = msg.success ? '' : 'err';
          const p = msg.payload || {};
          // 根据 request_id 查找对应的命令类型
          const cmd = getPendingMeta(msg.request_id).command || '';
          const cmdLabels = {
            'provider.update': '更新供应商响应',
            'provider.get': '获取供应商响应',
            'session.send': '发送消息响应',
            'session.messages': '消息历史响应',
            'session.get': '获取会话响应',
            'session.info': '会话详情响应',
            'session.delete': '删除会话响应',
            'session.clear': '清空上下文响应',
            'session.list': '会话列表响应',
            'tool.register': '注册工具响应',
            'tool.unregister': '注销工具响应',
            'system_prompt.get': '获取系统提示词响应',
            'system_prompt.set': '设置系统提示词响应',
            'tool.list': '工具列表响应',
            'tool.get': '工具详情响应',
            'runtime.sessions': '运行中会话响应',
            'system.stats': '系统统计响应',
            'context.preview': '上下文预览响应',
            'context.reset': '重置上下文响应',
            'context.exclude': '排除上下文响应',
            'context.include_after': '上下文起点响应',
            'context.keep_recent': '最近消息窗口响应',
            'context.seed.add': '注入上下文块响应',
            'context.compaction.apply': '压缩执行响应',
            'run.cancel': '取消推理响应',
          };
          let respLabel = cmdLabels[cmd] || '命令响应';
          if (cmd === 'session.send') {
            if (!msg.success) respLabel = '推理失败';
            else if (p.status === 'cancelled') respLabel = '推理中断';
            else if (p.status === 'completed') respLabel = '推理完成';
            else respLabel = '发送消息响应';
          }
          let brief = `${tag(msg.request_id)} ${tag(status, cls)}`;
          // 按命令类型提取关键信息
          if (p.provider) {
            const pr = p.provider;
            brief += ` ${tag((pr.protocol||'')+'/'+(pr.model||''))}`;
            if (pr.api_key) brief += ` key:${tag(pr.api_key.slice(0,8)+'...')}`;
            if (p.is_override !== undefined) brief += ` ${tag(p.is_override?'会话覆盖':'全局默认')}`;
          }
          if (p.messages) brief += ` 加载 ${p.messages.length} 条历史`;
          if (cmd === 'session.send') {
            if (p.run_id) brief += ` run ${tag((p.run_id || '').slice(0, 16))}`;
            if (p.status) brief += ` ${tag(p.status, p.status === 'cancelled' ? 'err' : '')}`;
            if (Array.isArray(p.trace_details) && p.trace_details.length) {
              const lastTrace = p.trace_details[p.trace_details.length - 1];
              const report = lastTrace?.tool_chain_report;
              brief += ` trace ${tag(p.trace_details.length)}`;
              if (report) brief += ` ${tag(toolChainStatusText(report), toolChainIssueCount(report) > 0 ? 'err' : '')}`;
            }
          }
          if (cmd === 'runtime.sessions') brief += ` session ${tag(p.running_session_count ?? 0)} run ${tag(p.running_run_count ?? 0)}`;
          if (p.active_context) {
            brief += ` ${tag(contextModeLabel(p.active_context.mode))}`;
            if (p.counts) brief += ` 可见${tag(p.counts.active_messages || 0)}条 / 全量${tag(p.counts.all_messages || 0)}条`;
          }
          if (p.seed) brief += ` ${tag(seedKindLabel(p.seed.kind))} ${tag((p.seed.seed_id || '').slice(0,16))}`;
          if (cmd === 'context.keep_recent' && p.active_context?.rules?.keep_recent_messages !== undefined) brief += ` 保留最近 ${tag(p.active_context.rules.keep_recent_messages || '全部')} 条`;
          if (cmd === 'context.include_after' && p.active_context?.rules?.include_after_message_id) brief += ` 起点 ${tag(p.active_context.rules.include_after_message_id.slice(0,16))}`;
          if (cmd === 'context.exclude' && p.active_context?.rules?.exclude_ranges) brief += ` 已排除 ${tag(p.active_context.rules.exclude_ranges.length)} 段`;
          if (p.content) brief += ` ${tag(JSON.stringify(p.content).slice(0,40))}`;
          if (p.session_id && !p.provider && !p.messages && !p.content && !p.active_context && !p.seed) brief += ` ${tag(p.session_id)}`;
          if (p.error) brief += ` ${tag(p.error.slice(0,40), 'err')}`;
          return { label: respLabel, brief };
        }
        // Event
        if (msg.type === 'event') {
          const et = msg.event_type || 'unknown';
          const p = msg.payload || {};
          const map = {
            'model.delta':              () => ({ label: p.event_type === 'thinking' ? '模型思维（流式）' : '模型输出（流式）', brief: `${p.event_type === 'thinking' ? '思维' : '文本'}增量 ${tag(JSON.stringify(p.delta||'').slice(0,40))}` }),
            'model.completed':          () => ({ label: '模型完成', brief: `${tag((p.content||'').slice(0,60))}` }),
            'tool_chain.diagnosed':     () => ({ label: '工具链诊断', brief: `${tag(toolChainStatusText(p.report), toolChainIssueCount(p.report) > 0 ? 'err' : '')} 完整 ${tag((p.report?.complete_tool_call_ids||[]).length)} 丢弃 ${tag(toolChainIssueCount(p.report))}` }),
            'session.created':          () => ({ label: '会话创建', brief: `标题 ${tag(p.title||'—')}` }),
            'session.closed':           () => ({ label: '会话关闭', brief: p.reason ? `原因 ${tag(p.reason)}` : '' }),
            'run.started':              () => ({ label: '推理开始', brief: `${tag(p.provider)} ${tag(p.model)}` }),
            'run.cancelled':            () => ({ label: '推理中断', brief: `${p.preserved ? tag('已保留部分输出') : tag('未保留输出')} ${tag(p.duration_ms ? p.duration_ms + 'ms' : '—')}` }),
            'run.completed':            () => ({ label: '推理完成', brief: `耗时 ${tag(p.duration_ms?p.duration_ms+'ms':'—')}` }),
            'tool.call.request':        () => ({ label: '工具执行请求', brief: `${tag(p.tool_name)} 输入 ${JSON.stringify(p.input||{}).slice(0,50)}` }),
            'tool.call.result':         () => ({ label: 'Core 已确认工具结果', brief: `${tag(p.tool_name)} 结果 ${(p.result||'').slice(0,40)} ${p.is_error?tag('错误','err'):tag('已接收')}` }),
            'tool.call.error':          () => ({ label: '工具调用失败', brief: `${tag(p.tool_name)} ${tag((p.error||'').slice(0,40),'err')}` }),
            'tool.registered':          () => ({ label: '工具注册', brief: `${tag(p.tool_name)} 客户端 ${tag(p.client_id)}` }),
            'context.threshold.reached':() => ({ label: '上下文阈值', brief: `使用 ${tag(p.usage_percent+'%')} Token ${tag(p.estimated_tokens)}` }),
            'context.updated':          () => ({ label: '上下文已更新', brief: `${tag(contextCommandName(p.action || 'context.updated'))} ${tag(contextModeLabel(p.context?.mode))}` }),
            'context.reset':            () => ({ label: '上下文已重置', brief: `${tag(contextModeLabel(p.context?.mode || 'full'))}` }),
            'context.seed.added':       () => ({ label: '上下文块已注入', brief: `${tag(seedKindLabel(p.seed?.kind))} ${tag((p.seed?.seed_id||'').slice(0,16))}` }),
            'context.compaction.applied':() =>({ label: '上下文压缩已应用', brief: `${tag(p.context?.context_id || '')}` }),
            'prompt.attached':          () => ({ label: 'Prompt 附加', brief: `${tag(p.prompt_name||p.name||'')}` }),
            'error':                    () => ({ label: '错误', brief: tag(JSON.stringify(p).slice(0,60), 'err') }),
          };
          const factory = map[et];
          if (factory) return factory();
          return { label: et, brief: JSON.stringify(p).slice(0, 60) };
        }
        // Stream
        if (msg.type === 'stream') {
          return { label: '流数据', brief: `事件 ${tag(msg.event)} ${JSON.stringify(msg.data||{}).slice(0,50)}` };
        }
        return { label: msg.type || '未知', brief: JSON.stringify(msg).slice(0, 60) };
      }

      // ─── 发出的消息 ───
      const cmd = msg.command || '';
      const sid = msg.session_id ? tag(msg.session_id) : '';
      const p = msg.payload || {};
      const cmdMap = {
        'session.send':         () => ({ label: '发送消息', brief: `${sid} ${tag(JSON.stringify(p.message||'').slice(0,50))}` }),
        'session.messages':     () => ({ label: '获取消息历史', brief: `${sid} 页${p.page||0} 每页${p.limit||50}` }),
        'session.get':          () => ({ label: '获取会话', brief: sid }),
        'session.info':         () => ({ label: '会话详情', brief: sid }),
        'session.delete':       () => ({ label: '删除会话', brief: sid }),
        'session.clear':        () => ({ label: '清空上下文', brief: sid }),
        'session.list':         () => ({ label: '列出会话', brief: `页${p.page||0} 每页${p.limit||20}` }),
        'provider.update':      () => ({ label: '更新供应商', brief: `${sid} ${tag(p.protocol)} ${tag(p.model)}` }),
        'provider.get':         () => ({ label: '获取供应商', brief: sid }),
        'tool.register':        () => ({ label: '注册工具', brief: `${tag(p.tool_name)} 客户端 ${tag(p.client_id||'')}` }),
        'tool.unregister':      () => ({ label: '注销工具', brief: tag(p.tool_name) }),
        'system_prompt.get':     () => ({ label: '获取系统提示词', brief: sid }),
        'system_prompt.set':     () => ({ label: '设置系统提示词', brief: `${sid} ${tag((p.system_prompt||'').length + ' 字')}` }),
        'tool.execute.result':  () => ({ label: '提交工具结果', brief: `${sid} call_id ${tag((p.call_id||'').slice(0,16))} ${p.is_error?tag('错误','err'):tag('成功')}` }),
        'tool.list':             () => ({ label: '获取工具列表', brief: sid }),
        'tool.get':              () => ({ label: '获取工具详情', brief: tag(p.tool_name) }),
        'runtime.sessions':      () => ({ label: '查询运行中会话', brief: '当前 runtime 状态' }),
        'system.stats':         () => ({ label: '系统统计', brief: '' }),
        'context.preview':      () => ({ label: '上下文预览', brief: sid }),
        'context.compaction.apply': () => ({ label: '执行压缩', brief: sid }),
        'run.cancel':           () => ({ label: '取消推理', brief: sid }),
      };
      const factory = cmdMap[cmd];
      if (factory) return factory();
      return { label: cmd || '未知命令', brief: sid || JSON.stringify(p).slice(0, 40) };
    }

    function addRawMessage(dir, data) {
      const { label, brief } = summarizeRaw(dir, data);

      // 记录发出的命令类型（用于响应中文标签）
      if (dir === 'out') {
        try {
          const out = JSON.parse(data);
          if (out.command && out.request_id) {
            // 保留已提前标记的特殊命令别名，例如 session.messages.full
            if (!pendingCommands.has(out.request_id)) {
              pendingCommands.set(out.request_id, {
                command: out.command,
                sessionId: out.session_id || '',
              });
            }
            // 清理旧记录（防止内存泄漏）
            if (pendingCommands.size > 200) {
              const keys = [...pendingCommands.keys()];
              keys.slice(0, 100).forEach(k => pendingCommands.delete(k));
            }
          }
        } catch {}
      }

      // 检测是否是连续的 model.delta 事件，如果是则合并到上一条
      if (dir === 'in') {
        let parsed;
        try { parsed = JSON.parse(data); } catch {}
        if (parsed && parsed.type === 'event' && parsed.event_type === 'model.delta') {
          const last = rawMessages.value[rawMessages.value.length - 1];
          if (last && last._merged) {
            // 合并到已有行
            last.count++;
            last.children.push({ data, time: now() });
            last.brief = `已接收 ${last.count} 条流式增量`;
            last.data = data; // 最新的 raw 作为主数据
            last.time = now();
            nextTick(() => { if (rawBody.value) rawBody.value.scrollTop = rawBody.value.scrollHeight; });
            return;
          }
          // 创建新的可合并行
          rawMessages.value.push({
            dir, data, time: now(), label, brief,
            _new: true, _expanded: false,
            _merged: true, count: 1,
            children: [{ data, time: now() }],
          });
          if (rawMessages.value.length > 500) rawMessages.value.splice(0, rawMessages.value.length - 500);
          nextTick(() => { if (rawBody.value) rawBody.value.scrollTop = rawBody.value.scrollHeight; });
          setTimeout(() => { const m = rawMessages.value[rawMessages.value.length-1]; if(m) m._new = false; }, 800);
          return;
        }
      }

      // 普通消息：直接追加
      rawMessages.value.push({ dir, data, time: now(), label, brief, _new: true, _expanded: false });
      if (rawMessages.value.length > 500) rawMessages.value.splice(0, rawMessages.value.length - 500);
      nextTick(() => { if (rawBody.value) rawBody.value.scrollTop = rawBody.value.scrollHeight; });
      setTimeout(() => { const m = rawMessages.value[rawMessages.value.length-1]; if(m) m._new = false; }, 800);
    }

    function scrollToChat() {
      nextTick(() => { if (chatBody.value) chatBody.value.scrollTop = chatBody.value.scrollHeight; });
    }

    const filteredEvents = computed(() => {
      if (eventFilter.value === 'all') return events.value;
      return events.value.filter(e => e.category === eventFilter.value);
    });

    const filteredRaw = computed(() => {
      if (rawFilter.value === 'all') return rawMessages.value;
      return rawMessages.value.filter(m => m.dir === rawFilter.value);
    });

    function saveProvider() {
      if (!ws) return;
      const rid = nextReqId();
      const payload = { ...providerConfig.value };
      // 去除空值
      if (!payload.api_key) delete payload.api_key;
      ws.send(JSON.stringify({
        command: 'provider.update',
        request_id: rid,
        session_id: sessionId.value,
        payload,
      }));
      addRawMessage('out', JSON.stringify({ command: 'provider.update', request_id: rid, session_id: sessionId.value, payload }));
      providerStatus.value = '已发送配置更新请求...';
      setTimeout(() => providerStatus.value = '', 3000);
    }

    function loadProvider() {
      if (!ws) return;
      const rid = nextReqId();
      ws.send(JSON.stringify({
        command: 'provider.get',
        request_id: rid,
        session_id: sessionId.value,
        payload: {},
      }));
      addRawMessage('out', JSON.stringify({ command: 'provider.get', request_id: rid, session_id: sessionId.value, payload: {} }));
    }

    function applyTemplate(name) {
      const templates = {
        deepseek: { protocol: 'openai', base_url: 'https://api.deepseek.com', model: 'deepseek-chat', max_tokens: 4096, temperature: 0 },
        openai:   { protocol: 'openai', base_url: 'https://api.openai.com', model: 'gpt-4o', max_tokens: 4096, temperature: 0 },
        claude:   { protocol: 'claude', base_url: 'https://ai.accbot.vip', model: 'claude-sonnet-4-20250514', max_tokens: 4096, temperature: 0 },
        ollama:   { protocol: 'openai', base_url: 'http://localhost:11434', model: 'qwen2.5:7b', max_tokens: 4096, temperature: 0 },
      };
      const t = templates[name];
      if (t) {
        Object.assign(providerConfig.value, t);
        saveProvider(); // 点击模板后自动保存并应用
      }
    }

    // 页面加载：自动连接（不做 localStorage 持久化，网页仅为调试用）
    onMounted(() => {
      if (wsUrl.value) connect();
      window.addEventListener('beforeunload', () => {
        if (ws) ws.close();
      });
    });

    return {
      wsUrl, connected, connectionId, sessionId,
      selectedSessionId, sessions,
      chatInput, chatMessages,
      fullMessages, fullMessagesPage, fullMessagesTotal, fullMessagesHasMore, fullMessagesLoading,
      runtimeSessions, runtimeRuns, runtimeRuntimeStatus, runtimeLoading,
      events, rawMessages, tools, toolExecLog, presetTools,
      rightTab, eventFilter, rawFilter, rawInput,
      chatBody, streamBody, rawBody, fullMessagesBody,
      providerConfig, providerStatus,
      systemPrompt, systemPromptStatus,
      contextPreview, contextStatus, contextKeepRecent, contextIncludeAfter,
      contextSeedKind, contextSeedContent, contextSummary,
      streamingText, isStreaming, currentRunId, currentRunStatus, runStatusLabel, autoReconnect,
      streamingThinking,
      latestToolChainReport, latestTraceDetails, formatToolChainIds, toolChainStatusText, toolChainIssueCount,
      connect, disconnect, sendChat, cancelCurrentRun, sendRaw, sendToolResult,
      defineClientTool, handleToolCallRequest,
      loadSessions, selectSession, createNewSession, deleteSession,
      registerPresetTool, registerPresetTools,
      saveProvider, loadProvider, applyTemplate,
      saveSystemPrompt, loadSystemPrompt, loadTools,
      loadRuntimeSessions,
      loadContextPreview, resetContext, keepRecentContext, includeAfterContext,
      excludeSingleMessage, addContextSeed, applyCompaction, contextMessageText,
      contextCommandName, contextModeLabel, seedKindLabel,
      loadFullMessages, onFullMessagesScroll, roleLabel, roleColor, formatMessageTime,
      filteredEvents, filteredRaw, statsExpanded, showPresetToolsModal, renderMarkdown,
      showApiKey, formatTimeoutLabel, askUserQuestionState,
    };
  }
}).mount('#app');
