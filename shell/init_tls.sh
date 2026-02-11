#!/usr/bin/env bash
installType='yum -y install'
upgrade="yum -y update"
echoType='echo -e'

# 打印
echoColor() {
  case $1 in
  # 红色
  "red")
    ${echoType} "\033[31m$2 \033[0m"
    ;;

  # 绿色
  "green")
    ${echoType} "\033[32m$2 \033[0m"
    ;;
  # 白色
  "white")
    ${echoType} "\033[37m$2 \033[0m"
    ;;
  "magenta")
    ${echoType} "\033[31m$2 \033[0m"
    ;;
  "skyBlue")
    ${echoType} "\033[36m$2 \033[0m"
    ;;
  # 黄色
  "yellow")
    ${echoType} "\033[33m$2 \033[0m"
    ;;
  esac
}
# 选择系统执行工具
checkSystem() {

  if [[ -n $(find /etc -name "redhat-release") ]] || grep -q -i "centos" /proc/version || grep -q -i "red hat" /proc/version || grep -q -i "redhat" /proc/version; then
    release="centos"
    installType='yum -y install'
    upgrade="yum update -y"
  elif grep -q -i "debian" /etc/issue || grep -q -i "debian" /proc/version; then
    release="debian"
    installType='apt -y install'
    upgrade="apt update -y"
  elif grep -q -i "ubuntu" /etc/issue || grep -q -i "ubuntu" /proc/version; then
    release="ubuntu"
    installType='apt -y install'
    upgrade="apt update -y"
  fi
  if [[ -z ${release} ]]; then
    echoContent red "本脚本不支持此系统，请将下方日志反馈给开发者"
    cat /etc/issue
    cat /proc/version
    exit 0
  fi
}
# 安装工具包
installTools() {
  echoColor yellow "更新"
  ${upgrade}
  if [[ -z $(find /usr/bin/ -executable -name "socat") ]]; then
    echoColor yellow "\nsocat未安装，安装中\n"
    ${installType} socat >/dev/null
    echoColor green "socat安装完毕"
  fi
  echoColor yellow "\n检测是否安装Nginx"
  if [[ -z $(find /sbin/ -executable -name 'nginx') ]]; then
    echoColor yellow "nginx未安装，安装中\n"
    ${installType} nginx >/dev/null
    echoColor green "nginx安装完毕"
  else
    echoColor green "nginx已安装\n"
  fi
  echoColor yellow "检测是否安装acme.sh"
  if [[ -z $(find ~/.acme.sh/ -name "acme.sh") ]]; then
    echoColor yellow "\nacme.sh未安装，安装中\n"
    curl -s https://get.acme.sh | sh >/dev/null
    echoColor green "acme.sh安装完毕\n"
  else
    echoColor green "acme.sh已安装\n"
  fi

}
# 恢复配置
resetNginxConfig() {
  cp -Rrf /tmp/wwps/nginx/nginx.conf /etc/nginx/nginx.conf
  rm -rf /etc/nginx/conf.d/5NX2O9XQKP.conf
  echoColor green "\n恢复配置完毕"
}
# 备份
bakConfig() {
  mkdir -p /tmp/youugiuhiuh/nginx
  cp -Rrf /etc/nginx/nginx.conf /tmp/wwps/nginx/nginx.conf
}
# 安装证书
installTLS() {
  echoColor yellow "请输入域名【例:blog.google.com】："
  read -r domain
  if [[ -z ${domain} ]]; then
    echoColor red "域名未填写\n"
    installTLS
  fi
  # 备份
  bakConfig
  # 替换原始文件中的域名
  if grep -q "${domain}" /etc/nginx/nginx.conf; then
    sed -i "s/${domain}/X655Y0M9UM9/g" "$(grep "${domain}" -rl /etc/nginx/nginx.conf)"
  fi

  touch /etc/nginx/conf.d/6GFV1ES52V2.conf
  echo "server {listen 80;server_name ${domain};root /usr/share/nginx/html;location ~ /.well-known {allow all;}location /test {return 200 '5NX2O9XQKP';}}" >/etc/nginx/conf.d/5NX2O9XQKP.conf
  nginxStatus=1
  if pgrep -x "nginx" >/dev/null; then
    nginxStatus=2
    pgrep -x "nginx" | xargs kill -9
    sleep 0.5
    nginx
  else
    nginx
  fi
  echoColor yellow "\n验证域名以及服务器是否可用"
  if curl -s "${domain}/test" | grep -q 5NX2O9XQKP; then
    pgrep -x "nginx" | xargs kill -9
    sleep 0.5
    echoColor green "服务可用，生成TLS中，请等待\n"
  else
    echoColor red "服务不可用请检测dns配置是否正确"
    # 恢复备份
    resetNginxConfig
    exit 0
  fi
  sudo ~/.acme.sh/acme.sh --issue -d "${domain}" --standalone -k ec-256 >/dev/null
  ~/.acme.sh/acme.sh --installcert -d "${domain}" --fullchainpath "/tmp/wwps/nginx/${domain}.crt" --keypath "/tmp/wwps/nginx/${domain}.key" --ecc >/dev/null
  if [[ ! -f "/tmp/wwps/nginx/${domain}.key" ]] || [[ -z $(cat "/tmp/wwps/nginx/${domain}.key") ]]; then
    echoColor red "证书key生成失败，请重新运行"
    resetNginxConfig
    exit
  elif [[ ! -f "/tmp/wwps/nginx/${domain}.crt" ]] || [[ -z $(cat "/tmp/wwps/nginx/${domain}.crt") ]]; then
    echoColor red "证书crt生成失败，请重新运行"
    resetNginxConfig
    exit
  fi
  echoColor green "证书生成成功"
  echoColor green "证书目录/tmp/youugiuhiuh/nginx"
  ls /tmp/youugiuhiuh/nginx

  resetNginxConfig
  if [[ ${nginxStatus} == 2 ]]; then
    nginx
  fi
}

init() {
  echoColor red "\n=============================="
  echoColor yellow "此脚本注意事项"
  echoColor green "   1.会安装依赖所需依赖"
  echoColor green "   2.会把Nginx配置文件备份"
  echoColor green "   3.会安装Nginx、acme.sh，如果已安装则使用已经存在的"
  echoColor green "   4.安装完毕或者安装失败会自动恢复备份，请不要手动关闭脚本"
  echoColor green "   5.执行期间请不要重启机器"
  echoColor green "   6.备份文件和证书文件都在/tmp下面，请注意留存"
  echoColor green "   7.如果多次执行则将上次生成备份和生成的证书强制覆盖"
  echoColor green "   8.证书默认ec-256"
  echoColor green "   9.下个版本会加入通配符证书生成[todo]"
  echoColor green "   10.可以生成多个不同域名的证书[包含子域名]，具体速率请查看[https://letsencrypt.org/zh-cn/docs/rate-limits/]"
  echoColor green "   11.兼容Centos、Ubuntu、Debian"
  echoColor green "   12.Github[https://github.com/youugiuhiuh/Wuthering_Waves_Private_Server]"
  echoColor red "=============================="
  echoColor yellow "请输入[y]执行脚本，[任意]结束:"
  read -r isExecStatus
  if [[ ${isExecStatus} == "y" ]]; then
    installTools
    installTLS
  else
    echoColor green "欢迎下次使用"
    exit
  fi
}
checkSystem
init
