# lane

Home's batch lane (paulgsc/server#705, L4 #708): the decision of whether the
lowest class of work, the renders, may run now, must pause, or must stop and
let go of its memory. It is a pure function of the kernel's own signals
(`/proc/meminfo`, `/proc/pressure/*`) and of whether interactive work is
waiting; no I/O happens here. file_host is to read the signals and write
the decision, and the render worker to obey it (L4b, L4c on #708). `docs/home-compute.md` is the policy.
