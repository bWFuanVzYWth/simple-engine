//! 真实原子余量与线性票据；不把不重复发行设为外部假设。
use super::*;
use crate::identity_ops::model::{self, Permission};
use vstd::{invariant::InvariantPredicate, prelude::*};
verus! {
impl crate::identity_ops::StepModel for Checked {
    open spec fn next(value:usize)->Option<usize> {if value<usize::MAX {Some((value+1) as usize)}else{None}}
}
pub(crate) type Pair = (Permission, GhostSetAuth<usize>);
pub(crate) const fn empty<T>() -> (result:Tracked<Option<T>>)
    ensures result@ is None,
{ let tracked none=None; Tracked(none) }
pub(crate) const fn initialize(value:&AtomicUsize,Tracked(permission):Tracked<Option<Permission>>)->(result:Tracked<AtomicInvariant<(int,vstd::resource::Loc),Pair,Pred>>)
    requires permission is Some,model::view(permission->Some_0).0==model::cell(value),model::view(permission->Some_0).1>=1,
    ensures result@.constant().0==model::cell(value),
{
    let tracked permission=permission.tracked_unwrap();
    let ghost initial=model::view(permission).1;
    let tracked (auth,_)=GhostSetAuth::<usize>::new(Set::<usize>::range(1usize,initial));
    let tracked inv=AtomicInvariant::new((model::cell(value),auth.id()),(permission,auth),0);
    Tracked(inv)
}
pub(crate) fn unpack(Tracked(ticket):Tracked<Option<Ticket>>)->(result:Tracked<Ticket>)
    requires ticket is Some,
    ensures result@==ticket->Some_0,
{ Tracked(ticket.tracked_unwrap()) }
pub(crate) struct Pred;
impl InvariantPredicate<(int,vstd::resource::Loc),Pair> for Pred {
    open spec fn inv(k:(int,vstd::resource::Loc),pair:Pair)->bool {
        model::view(pair.0).0==k.0 && pair.1.id()==k.1 && 1<=model::view(pair.0).1
            && pair.1@==Set::<usize>::range(1usize,model::view(pair.0).1)
    }
}
impl Counter {
    pub(crate) open(crate) spec fn valid(&self)->bool { self.inv@.constant().0==model::cell(&self.value) }
    pub(crate) open(crate) spec fn authority(&self)->vstd::resource::Loc {self.inv@.constant().1}
}
pub(crate) proof fn issued(result:Result<usize,usize>,tracked auth:&mut GhostSetAuth<usize>,permission:Permission,tracked out:&mut Tracked<Option<Ticket>>)
    requires 1<=model::view(permission).1,
        match result {
            Ok(value)=>1<=value<usize::MAX && model::view(permission).1==value+1 && old(auth)@==Set::<usize>::range(1usize,value),
            Err(value)=>value==usize::MAX && model::view(permission).1==value && old(auth)@==Set::<usize>::range(1usize,value),
        },
    ensures final(auth).id()==old(auth).id(),final(auth)@==Set::<usize>::range(1usize,model::view(permission).1),
        result is Ok ==> final(out)@ is Some && final(out)@->Some_0@==result->Ok_0 && final(out)@->Some_0.id()==final(auth).id(),
        result is Err ==> final(out)@==old(out)@,
{
    if let Ok(value)=result {
        vstd::set_lib::range_set_properties(1usize,value);
        let tracked ticket=auth.insert(value);
        *out.borrow_mut()=Some(ticket);
        vstd::set_lib::range_set_properties(1usize,model::view(permission).1);
        assert(auth@ =~= Set::<usize>::range(1usize,model::view(permission).1));
    }
}
pub open spec fn ticket_valid(ticket:Ticket,owner:usize)->bool {
    ticket@==owner && ticket.id()==model::global_authority()
}
impl Owner {
    pub open(crate) spec fn valid(&self)->bool {ticket_valid(self.ticket@,self.number)}
    pub open(crate) spec fn number_spec(&self)->usize {self.number}
}
}
