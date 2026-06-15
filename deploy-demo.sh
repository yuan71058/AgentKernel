#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_SCRIPT="${ROOT_DIR}/build.sh"

# 复用原脚本中的服务器账号信息
DEPLOY_HOST="your server host"
DEPLOY_USER="your server user"
DEPLOY_PASS="your server pass"
DEPLOY_PORT="22"

DEPLOY_PATH="/opt/agentkernel"
DEPLOY_BINARY_NAME="agentkernel"
DEPLOY_LISTEN_ADDR="0.0.0.0:9991"
DEPLOY_SERVER_PORT="9991"
DEPLOY_PROTOCOL=""
DEPLOY_MODEL=""
DEPLOY_BASE_URL=""
DEPLOY_API_KEY=""
DEPLOY_SYSTEM_PROMPT=""

LOCAL_FILE="${ROOT_DIR}/dist/${DEPLOY_BINARY_NAME}-linux-amd64"
REMOTE_TMP_FILE="/tmp/${DEPLOY_BINARY_NAME}-linux-amd64"
REMOTE_LOG_DIR="${DEPLOY_PATH}/logs"
REMOTE_LOG_FILE="${REMOTE_LOG_DIR}/${DEPLOY_BINARY_NAME}.log"
REMOTE_DATA_DIR="${DEPLOY_PATH}/data"
REMOTE_SUDO="echo '${DEPLOY_PASS}' | sudo -S"

usage() {
  cat <<EOF
用法: $0 [选项]

选项:
  --path <dir>            远程部署目录，默认: ${DEPLOY_PATH}
  --addr <host:port>      程序监听地址，默认: ${DEPLOY_LISTEN_ADDR}
  --protocol <name>       可选，传给 agentkernel 的 --protocol
  --model <name>          可选，传给 agentkernel 的 --model
  --base-url <url>        可选，传给 agentkernel 的 --base-url
  --api-key <key>         可选，传给 agentkernel 的 --api-key
  --system-prompt <text>  可选，传给 agentkernel 的 --system-prompt
  -h, --help              显示帮助
EOF
}

log() {
  printf '\n[%s] %s\n' "$(date '+%H:%M:%S')" "$*"
}

shell_quote() {
  printf '%q' "$1"
}

expect_scp() {
  local source_file="$1"
  local target_path="$2"
  local scp_cmd
  scp_cmd="scp -o StrictHostKeyChecking=no -P ${DEPLOY_PORT} $(shell_quote "${source_file}") $(shell_quote "${DEPLOY_USER}@${DEPLOY_HOST}:${target_path}")"
  /usr/bin/expect <<EOF
set timeout -1
spawn sh -lc "${scp_cmd}"
expect {
    "*yes/no*" { send "yes\r"; exp_continue }
    "*assword:*" { send "${DEPLOY_PASS}\r"; exp_continue }
    eof
}
catch wait result
set code [lindex \$result 3]
exit \$code
EOF
}

expect_ssh_script() {
  local script_file="$1"
  local ssh_cmd
  ssh_cmd="ssh -o StrictHostKeyChecking=no -p ${DEPLOY_PORT} $(shell_quote "${DEPLOY_USER}@${DEPLOY_HOST}") 'bash -s' < $(shell_quote "${script_file}")"
  /usr/bin/expect <<EOF
set timeout -1
spawn sh -lc "${ssh_cmd}"
expect {
    "*yes/no*" { send "yes\r"; exp_continue }
    "*assword:*" { send "${DEPLOY_PASS}\r"; exp_continue }
    eof
}
catch wait result
set code [lindex \$result 3]
exit \$code
EOF
}

append_optional_arg() {
  local option_name="$1"
  local option_value="$2"
  if [[ -n "${option_value}" ]]; then
    RUN_ARGS+=" ${option_name} $(shell_quote "${option_value}")"
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --path)
      DEPLOY_PATH="$2"
      shift 2
      ;;
    --addr)
      DEPLOY_LISTEN_ADDR="$2"
      shift 2
      ;;
    --protocol)
      DEPLOY_PROTOCOL="$2"
      shift 2
      ;;
    --model)
      DEPLOY_MODEL="$2"
      shift 2
      ;;
    --base-url)
      DEPLOY_BASE_URL="$2"
      shift 2
      ;;
    --api-key)
      DEPLOY_API_KEY="$2"
      shift 2
      ;;
    --system-prompt)
      DEPLOY_SYSTEM_PROMPT="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "❌ 未知参数: $1"
      usage
      exit 1
      ;;
  esac
done

REMOTE_LOG_DIR="${DEPLOY_PATH}/logs"
REMOTE_LOG_FILE="${REMOTE_LOG_DIR}/${DEPLOY_BINARY_NAME}.log"
REMOTE_DATA_DIR="${DEPLOY_PATH}/data"

RUN_ARGS="--addr $(shell_quote "${DEPLOY_LISTEN_ADDR}") --data-dir $(shell_quote "${REMOTE_DATA_DIR}")"
append_optional_arg "--protocol" "${DEPLOY_PROTOCOL}"
append_optional_arg "--model" "${DEPLOY_MODEL}"
append_optional_arg "--base-url" "${DEPLOY_BASE_URL}"
append_optional_arg "--api-key" "${DEPLOY_API_KEY}"
append_optional_arg "--system-prompt" "${DEPLOY_SYSTEM_PROMPT}"

if [[ "${DEPLOY_LISTEN_ADDR}" =~ :([0-9]+)$ ]]; then
  DEPLOY_SERVER_PORT="${BASH_REMATCH[1]}"
else
  echo "❌ 无法从监听地址解析端口: ${DEPLOY_LISTEN_ADDR}"
  exit 1
fi

log "步骤 1: 编译 Linux 版本"
bash "${BUILD_SCRIPT}" --linux

if [[ ! -f "${LOCAL_FILE}" ]]; then
  echo "❌ 未找到编译产物: ${LOCAL_FILE}"
  exit 1
fi

log "步骤 2: 上传二进制到 ${DEPLOY_USER}@${DEPLOY_HOST}:${REMOTE_TMP_FILE}"
expect_scp "${LOCAL_FILE}" "${REMOTE_TMP_FILE}"

log "步骤 3: 远程替换旧版本并重启服务"
REMOTE_SCRIPT=$(cat <<EOF
set -euo pipefail

if ! sudo -n true 2>/dev/null; then
  if ! ${REMOTE_SUDO} true 2>/dev/null; then
    echo "sudo 验证失败，请检查密码是否正确或是否有 sudo 权限。" >&2
    exit 1
  fi
fi

${REMOTE_SUDO} mkdir -p "${DEPLOY_PATH}" "${REMOTE_LOG_DIR}" "${REMOTE_DATA_DIR}"
${REMOTE_SUDO} chown -R "${DEPLOY_USER}:${DEPLOY_USER}" "${REMOTE_LOG_DIR}" "${REMOTE_DATA_DIR}"

${REMOTE_SUDO} pkill -9 -f "${DEPLOY_BINARY_NAME}" || true
port_pid=\$(${REMOTE_SUDO} lsof -t -i:${DEPLOY_SERVER_PORT} || true)
if [[ -n "\${port_pid}" ]]; then
  echo "发现端口 ${DEPLOY_SERVER_PORT} 被进程 \${port_pid} 占用，正在强制结束..."
  ${REMOTE_SUDO} kill -9 \${port_pid} || true
fi

if [[ -f "${DEPLOY_PATH}/${DEPLOY_BINARY_NAME}" ]]; then
  ${REMOTE_SUDO} cp "${DEPLOY_PATH}/${DEPLOY_BINARY_NAME}" "${DEPLOY_PATH}/${DEPLOY_BINARY_NAME}.bak"
fi

${REMOTE_SUDO} install -o "${DEPLOY_USER}" -g "${DEPLOY_USER}" -m 755 "${REMOTE_TMP_FILE}" "${DEPLOY_PATH}/${DEPLOY_BINARY_NAME}"
rm -f "${REMOTE_TMP_FILE}"
touch "${REMOTE_LOG_FILE}"

cd "${DEPLOY_PATH}"
nohup "./${DEPLOY_BINARY_NAME}" ${RUN_ARGS} > "${REMOTE_LOG_FILE}" 2>&1 </dev/null &

sleep 3
if ! pgrep -af "${DEPLOY_BINARY_NAME}" >/dev/null; then
  echo "远程启动失败，最近日志："
  tail -n 50 "${REMOTE_LOG_FILE}" || true
  exit 1
fi

listen_pid=\$(${REMOTE_SUDO} lsof -t -i:${DEPLOY_SERVER_PORT} -sTCP:LISTEN || true)
if [[ -z "\${listen_pid}" ]]; then
  echo "服务进程已启动，但端口 ${DEPLOY_SERVER_PORT} 未处于监听状态，最近日志："
  tail -n 50 "${REMOTE_LOG_FILE}" || true
  exit 1
fi

pgrep -af "${DEPLOY_BINARY_NAME}"
${REMOTE_SUDO} lsof -nP -iTCP:${DEPLOY_SERVER_PORT} -sTCP:LISTEN || true
EOF
)

TEMP_SCRIPT="$(mktemp)"
trap 'rm -f "${TEMP_SCRIPT}"' EXIT
printf '%s\n' "${REMOTE_SCRIPT}" > "${TEMP_SCRIPT}"
expect_ssh_script "${TEMP_SCRIPT}"
rm -f "${TEMP_SCRIPT}"
trap - EXIT

log "✅ 部署完成"
log "二进制: ${LOCAL_FILE}"
log "远程目录: ${DEPLOY_PATH}"
log "日志查看: ssh ${DEPLOY_USER}@${DEPLOY_HOST} 'tail -f ${REMOTE_LOG_FILE}'"
