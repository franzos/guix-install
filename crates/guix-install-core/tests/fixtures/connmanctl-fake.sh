#!/bin/sh
# Fake connmanctl for tests. Behavior controlled by env:
#   FAKE_STATE  : value printed for `state` (default "online")
case "$1" in
  state)        echo "  State = ${FAKE_STATE:-online}" ;;
  connect)
    if [ -n "$FAKE_CONNECT_EXIT" ]; then exit "$FAKE_CONNECT_EXIT"; fi
    echo "Connected $2" ;;
  enable|scan)  : ;;
  services)     echo "*AO Net   wifi_x_managed_psk" ;;
  technologies) printf '/net/connman/technology/wifi\n  Powered = True\n' ;;
  *)            : ;;
esac
exit 0
