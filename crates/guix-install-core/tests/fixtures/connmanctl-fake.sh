#!/bin/sh
# Fake connmanctl for tests. Behavior controlled by env:
#   FAKE_STATE          : value printed for global `state` (default "online")
#   FAKE_SERVICE_STATE  : value printed for `services <path>` detail (default "online")
#   FAKE_SERVICE_ERROR  : if set, printed as `Error =` in `services <path>` detail
case "$1" in
  state)        echo "  State = ${FAKE_STATE:-online}" ;;
  connect)
    if [ -n "$FAKE_CONNECT_EXIT" ]; then exit "$FAKE_CONNECT_EXIT"; fi
    echo "Connected $2" ;;
  enable|scan)  : ;;
  services)
    if [ -n "$2" ]; then
      echo "  State = ${FAKE_SERVICE_STATE:-online}"
      if [ -n "$FAKE_SERVICE_ERROR" ]; then echo "  Error = ${FAKE_SERVICE_ERROR}"; fi
    else
      echo "*AO Net   wifi_x_managed_psk"
    fi ;;
  technologies) printf '/net/connman/technology/wifi\n  Powered = True\n' ;;
  *)            : ;;
esac
exit 0
