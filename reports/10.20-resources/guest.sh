#!/system/bin/sh
# The guest loop of reports/10.20-resources.md: CPU and PSS of the tools'
# processes in an Android emulator's guest, and every process's CPU.
#
# sampler.py --emulator <serial> pushes this file to /data/local/tmp/res-guest.sh
# and runs it as root, which reads every process's smaps_rollup (a userdebug
# image's su allows it), stamping each line with the host's time as it arrives:
#
#   adb -s <serial> shell "su 0 sh /data/local/tmp/res-guest.sh INTERVAL DISCOVER_EVERY 'PATTERN'"
#
# with INTERVAL 0.5, DISCOVER_EVERY 4 and PATTERN
# 'mobile-agent|maestro|agentdevice|dev.mobileagent.fixture|instrument' in the
# published runs. It loops until killed.
#
# Every INTERVAL seconds:
#   T tick uptime  cpu-line fields of /proc/stat (user nice system idle iowait irq softirq steal)
#   S tick pid utime stime starttime rss_pages pss_kB swappss_kB   (ticks of 1/100 s)
#   M tick utime stime cutime cstime                                (this loop and its children)
# Every DISCOVER_EVERY ticks, every process's accumulated CPU, then the
# processes whose name or arguments match PATTERN:
#   A tick pid ppid user time+ name
#   P tick pid ppid user name args
IV=${1:-0.5}
DE=${2:-2}
PAT=${3:-mobile-agent|maestro|agentdevice|dev.mobileagent.fixture|instrument}
n=0
PIDS=""
while :; do
  read -r up idle < /proc/uptime
  read -r cpu u ni sy id io irq si st rest < /proc/stat
  echo "T $n $up $u $ni $sy $id $io $irq $si $st"
  if [ $((n % DE)) -eq 0 ]; then
    ps -A -o PID=,PPID=,USER=,TIME+=,NAME= | sed "s/^ */A $n /"
    out=$(ps -A -o PID=,PPID=,USER=,NAME=,ARGS= | grep -E "$PAT" | grep -v -E "grep -E|res-guest")
    if [ -n "$out" ]; then
      echo "$out" | while read -r pid ppid user name args; do echo "P $n $pid $ppid $user $name $args"; done
      PIDS=$(echo "$out" | while read -r pid rest2; do echo "$pid"; done)
    else
      PIDS=""
    fi
  fi
  for p in $PIDS; do
    if read -r line < /proc/$p/stat 2>/dev/null; then
      f=${line##*) }
      set -- $f
      ut=${12}; stm=${13}; stt=${20}; rss=${22}
      pss=""; sw=""
      while read -r k v unit; do
        case $k in
          Pss:) pss=$v ;;
          SwapPss:) sw=$v ;;
        esac
      done < /proc/$p/smaps_rollup
      echo "S $n $p $ut $stm $stt $rss $pss $sw"
    fi
  done
  read -r line < /proc/$$/stat
  f=${line##*) }
  set -- $f
  echo "M $n ${12} ${13} ${14} ${15}"
  n=$((n+1))
  sleep "$IV"
done
