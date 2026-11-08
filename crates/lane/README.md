# lane

Home's batch lane (paulgsc/server#705, L4 #708): the decision of whether the
lowest class of work, the renders, may run now, must pause, or must stop and
let go of its memory. It is a pure function of the kernel's own signals
(`/proc/meminfo`, `/proc/pressure/*`) and of whether interactive work is
waiting; no I/O happens here. file_host reads the signals and writes the
decision; the render worker obeys it. `docs/home-compute.md` is the policy.
