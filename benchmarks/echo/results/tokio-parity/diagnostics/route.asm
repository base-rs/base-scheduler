 Percent |	Source code & Disassembly of exclusive for cycles:u (641 samples, percent: local period)
--------------------------------------------------------------------------------------------------------
         :
         :
         :
         : 3     Disassembly of section .text:
         :
         : 5     00000000000ff110 <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route>:
         : 6     } else {
         : 7     Ok(())
         : 8     }
         : 9     }
         :
         : 11    fn route(&self) -> Arc<Controller> {
    0.00 :   ff110:  push   %r14
    0.00 :   ff112:  push   %rbx
    0.00 :   ff113:  push   %rax
    0.00 :   ff114:  mov    %rdi,%rbx
    0.00 :   ff117:  mov    0x18(%rdi),%r14
    0.00 :   ff11b:  xor    %edi,%edi
    0.00 :   ff11d:  call   112770 <<core_scheduler_iouring::io_uring_worker::ROUTES::{K#0}::{closure#0} as core::ops::function::FnOnce<(core::option::Option<&mut core::option::Option<core::cell::RefCell<alloc::vec::Vec<alloc::rc::Rc<core_scheduler_iouring::io_uring_worker::LocalRoute>>>>>,)>>::call_once>
    0.00 :   ff122:  test   %rax,%rax
    0.00 :   ff125:  je     ff1ec <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route+0xdc>
    0.00 :   ff12b:  mov    (%rax),%rcx
    0.00 :   ff12e:  movabs $0x7fffffffffffffff,%rdx
    0.00 :   ff138:  cmp    %rdx,%rcx
    0.00 :   ff13b:  jae    ff1df <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route+0xcf>
    0.00 :   ff141:  lea    0x1(%rcx),%rdx
    0.00 :   ff145:  mov    %rdx,(%rax)
    0.00 :   ff148:  mov    0x18(%rax),%rdx
    0.16 :   ff14c:  test   %rdx,%rdx
    0.00 :   ff14f:  je     ff171 <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route+0x61>
    0.00 :   ff151:  mov    0x10(%rax),%rsi
    0.00 :   ff155:  shl    $0x3,%rdx
    0.00 :   ff159:  nopl   0x0(%rax)
    0.00 :   ff160:  mov    -0x8(%rsi,%rdx,1),%rdi
    0.31 :   ff165:  cmp    %r14,0x10(%rdi)
    0.47 :   ff169:  je     ff1a9 <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route+0x99>
    0.00 :   ff16b:  add    $0xfffffffffffffff8,%rdx
    0.00 :   ff16f:  jne    ff160 <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route+0x50>
    0.00 :   ff171:  mov    %rcx,(%rax)
    0.00 :   ff174:  mov    $0x1,%eax
    0.00 :   ff179:  lock xadd %rax,0x28(%rbx)
    0.00 :   ff17f:  mov    0x10(%rbx),%rsi
         : 42    crate::io_uring_worker::route(&self.group).unwrap_or_else(|| {
         : 43    let index = self.external_next.fetch_add(1, Ordering::Relaxed) % self.controllers.len();
    0.00 :   ff183:  test   %rsi,%rsi
    0.00 :   ff186:  je     ff1fb <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route+0xeb>
    0.00 :   ff188:  mov    0x8(%rbx),%rcx
    0.00 :   ff18c:  mov    %rax,%rdx
    0.00 :   ff18f:  or     %rsi,%rdx
    0.00 :   ff192:  shr    $0x20,%rdx
    0.00 :   ff196:  je     ff1c5 <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route+0xb5>
    0.00 :   ff198:  xor    %edx,%edx
    0.00 :   ff19a:  div    %rsi
    0.00 :   ff19d:  mov    (%rcx,%rdx,8),%rax
    0.00 :   ff1a1:  lock incq (%rax)
    0.00 :   ff1a5:  jg     ff1d3 <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route+0xc3>
    0.00 :   ff1a7:  jmp    ff1f9 <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route+0xe9>
    0.00 :   ff1a9:  mov    0x18(%rdi),%rcx
    0.00 :   ff1ad:  lock incq (%rcx)
   99.07 :   ff1b1:  jle    ff1f9 <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route+0xe9>
    0.00 :   ff1b3:  mov    0x18(%rdi),%rcx
    0.00 :   ff1b7:  decq   (%rax)
    0.00 :   ff1ba:  mov    %rcx,%rax
         : 63    Arc::clone(&self.controllers[index])
         : 64    })
         : 65    }
    0.00 :   ff1bd:  add    $0x8,%rsp
    0.00 :   ff1c1:  pop    %rbx
    0.00 :   ff1c2:  pop    %r14
    0.00 :   ff1c4:  ret
         : 70    let index = self.external_next.fetch_add(1, Ordering::Relaxed) % self.controllers.len();
    0.00 :   ff1c5:  xor    %edx,%edx
    0.00 :   ff1c7:  div    %esi
    0.00 :   ff1c9:  mov    (%rcx,%rdx,8),%rax
    0.00 :   ff1cd:  lock incq (%rax)
    0.00 :   ff1d1:  jle    ff1f9 <<core_scheduler_iouring::io_uring_selector::IoUringSelector>::route+0xe9>
    0.00 :   ff1d3:  mov    (%rcx,%rdx,8),%rax
         : 77    }
    0.00 :   ff1d7:  add    $0x8,%rsp
    0.00 :   ff1db:  pop    %rbx
    0.00 :   ff1dc:  pop    %r14
    0.00 :   ff1de:  ret
    0.00 :   ff1df:  lea    0xa03a2(%rip),%rdi        # 19f588 <anon.a53b6018d54340b37cc8ccb19e6526d1.7.llvm.12045270258440092400>
    0.00 :   ff1e6:  call   *0xa5afc(%rip)        # 1a4ce8 <_DYNAMIC+0xc90>
    0.00 :   ff1ec:  lea    0xa031d(%rip),%rdi        # 19f510 <anon.a53b6018d54340b37cc8ccb19e6526d1.1.llvm.12045270258440092400>
    0.00 :   ff1f3:  call   *0xa53cf(%rip)        # 1a45c8 <_DYNAMIC+0x570>
         : 86    crate::io_uring_worker::route(&self.group).unwrap_or_else(|| {
    0.00 :   ff1f9:  ud2
         : 88    let index = self.external_next.fetch_add(1, Ordering::Relaxed) % self.controllers.len();
    0.00 :   ff1fb:  lea    0x9f496(%rip),%rdi        # 19e698 <anon.24227df9b717c07173d5e52985423059.4.llvm.2827690211868720449+0x18>
    0.00 :   ff202:  call   *0xa5a60(%rip)        # 1a4c68 <_DYNAMIC+0xc10>
