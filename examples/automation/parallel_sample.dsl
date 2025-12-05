# DSL demo: parallel navigation branches
set home https://example.com
set docs https://www.iana.org/domains/example

navigate {{home}}
parallel 2
  branch
    click a.more-info
    wait 300
  endbranch
  branch
    navigate {{docs}}
    wait 500
  endbranch
endparallel
screenshot parallel-result.png
