export LD_PRELOAD=/test/memsocket/memsocket.so
/usr/local/apache2/bin/httpd -f /benchmark/apache/httpd.conf -D FOREGROUND &
/benchmark/bin/wrk -c 1 -t 1 -d 180s http://127.0.0.1:8080/index.html