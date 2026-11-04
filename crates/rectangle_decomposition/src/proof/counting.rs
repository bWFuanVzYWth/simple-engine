//! 有限整数求和与双重计数；全部递归均按有限长度下降。

use vstd::prelude::*;

verus! {

pub(crate) open spec fn bit(p: bool) -> int { if p { 1 } else { 0 } }

pub(crate) open spec fn sum(n: nat, f: spec_fn(int) -> int) -> int
    decreases n,
{
    if n == 0 { 0 } else { sum((n - 1) as nat, f) + f(n - 1) }
}

pub(crate) proof fn sum_equal(n: nat, f: spec_fn(int) -> int, g: spec_fn(int) -> int)
    requires forall|i: int| 0 <= i < n ==> #[trigger] f(i) == #[trigger] g(i),
    ensures sum(n, f) == sum(n, g),
    decreases n,
{
    if n > 0 { sum_equal((n - 1) as nat, f, g); }
}

pub(crate) proof fn sum_add(n: nat, f: spec_fn(int) -> int, g: spec_fn(int) -> int)
    ensures sum(n, |i: int| f(i) + g(i)) == sum(n, f) + sum(n, g),
    decreases n,
{
    if n > 0 { sum_add((n - 1) as nat, f, g); }
}

pub(crate) proof fn sum_scale(n: nat, k: int, f: spec_fn(int) -> int)
    ensures sum(n, |i: int| k * f(i)) == k * sum(n, f),
    decreases n,
{
    reveal_with_fuel(sum, 2);
    if n > 0 {
        sum_scale((n - 1) as nat, k, f);
        assert(k * (sum((n - 1) as nat, f) + f(n - 1))
            == k * sum((n - 1) as nat, f) + k * f(n - 1)) by(nonlinear_arith);
    }
}

pub(crate) proof fn sum_constant(n: nat, k: int)
    ensures sum(n, |_: int| k) == n * k,
    decreases n,
{
    if n > 0 {
        sum_constant((n - 1) as nat, k);
        assert((n - 1) * k + k == n * k) by(nonlinear_arith);
        assert(sum(n, |_: int| k) == sum((n-1) as nat, |_: int| k) + k);
    } else {
        assert(n*k == 0) by(nonlinear_arith) requires n == 0;
    }
}

pub(crate) proof fn sum_nonnegative(n: nat, f: spec_fn(int) -> int)
    requires forall|i: int| 0 <= i < n ==> #[trigger] f(i) >= 0,
    ensures sum(n, f) >= 0,
    decreases n,
{
    if n > 0 { sum_nonnegative((n - 1) as nat, f); }
}

/// 不建立物化矩阵；交换同一有限函数的两个求和次序。
pub(crate) proof fn sum_swap(m: nat, n: nat, f: spec_fn(int, int) -> int)
    ensures sum(m, |i: int| sum(n, |j: int| f(i, j)))
        == sum(n, |j: int| sum(m, |i: int| f(i, j))),
    decreases m,
{
    if m > 0 {
        assert(sum(m, |i: int| sum(n, |j: int| f(i,j)))
            == sum((m-1) as nat, |i: int| sum(n, |j: int| f(i,j)))
                + sum(n, |j: int| f(m-1,j)));
        sum_swap((m - 1) as nat, n, f);
        sum_add(n, |j: int| sum((m - 1) as nat, |i: int| f(i, j)),
            |j: int| f(m - 1, j));
        let g = |j: int| sum((m - 1) as nat, |i: int| f(i,j));
        let h = |j: int| f(m - 1,j);
        sum_equal(n, |j: int| g(j)+h(j),
            |j: int| sum((m - 1) as nat, |i: int| f(i,j))+f(m-1,j));
        sum_equal(n, |j: int| sum(m, |i: int| f(i, j)),
            |j: int| sum((m - 1) as nat, |i: int| f(i, j)) + f(m - 1, j));
        assert(sum((m-1) as nat, |i: int| sum(n, |j: int| f(i,j)))
            == sum(n, |j: int| sum((m-1) as nat, |i: int| f(i,j))));
        assert(sum(n, |j: int| sum((m-1) as nat, |i: int| f(i,j)) + f(m-1,j))
            == sum(n, |j: int| sum((m-1) as nat, |i: int| f(i,j)))
                + sum(n, |j: int| f(m-1,j)));
        assert(sum(m, |i: int| sum(n, |j: int| f(i,j)))
            == sum(n, |j: int| sum(m, |i: int| f(i,j))));
    } else {
        sum_constant(n, 0);
        sum_equal(n, |j: int| sum(m, |i: int| f(i,j)), |_: int| 0);
        assert(sum(m, |i: int| sum(n, |j: int| f(i,j)))
            == sum(n, |j: int| sum(m, |i: int| f(i,j))));
    }
}

/// 一项为 1、其余为 0 的有限和。
pub(crate) proof fn sum_single(n: nat, k: int)
    ensures sum(n, |i: int| bit(i == k)) == bit(0 <= k < n),
    decreases n,
{
    if n > 0 { sum_single((n - 1) as nat, k); }
}

pub(crate) proof fn sum_four_groups(m: nat, f: spec_fn(int) -> int)
    ensures sum(4*m, f) == sum(m, |v: int| sum(4, |q: int|f(4*v+q))),
    decreases m,
{
    if m>0 {
        sum_four_groups((m-1) as nat,f);
        reveal_with_fuel(sum,5);
    }
}

pub(crate) proof fn sum_corner_identity(n: nat, w: spec_fn(int)->int, c: spec_fn(int)->int,
    r: spec_fn(int)->int, e: spec_fn(int)->int, x: spec_fn(int)->int)
    requires forall|i: int| 0<=i<n ==> #[trigger] w(i)==#[trigger] c(i)
        +4*#[trigger] r(i)-2*#[trigger] e(i)-4*#[trigger] x(i),
    ensures sum(n,w)==sum(n,c)+4*sum(n,r)-2*sum(n,e)-4*sum(n,x),
    decreases n,
{
    if n>0 { sum_corner_identity((n-1) as nat,w,c,r,e,x); }
}

pub(crate) proof fn sum_concat(a: nat, b: nat, f: spec_fn(int)->int)
    ensures sum(a+b,f)==sum(a,f)+sum(b,|i: int|f(a+i)),
    decreases b,
{
    if b>0 { sum_concat(a,(b-1) as nat,f); }
}

pub(crate) proof fn sum_blocks(m: nat, n: nat, f: spec_fn(int)->int)
    ensures sum(m*n,f)==sum(m,|i: int|sum(n,|j: int|f(i*n+j))),
    decreases m,
{
    if m>0 {
        sum_blocks((m-1) as nat,n,f);
        sum_concat(((m-1)*n) as nat,n,f);
        assert((m-1)*n+n==m*n) by(nonlinear_arith);
    } else {
        assert(m*n==0) by(nonlinear_arith) requires m==0;
    }
}

/// 相邻项差之和，包含两端；无需枚举颜色集合。
pub(crate) proof fn telescope(n: nat, f: spec_fn(int)->int)
    ensures sum(n,|i: int|f(i)-f(i-1)) == (if n==0 { 0 } else { f(n-1)-f(-1) }),
    decreases n,
{
    if n>0 { telescope((n-1) as nat,f); }
}

pub(crate) proof fn grid_sum(m: nat,n: nat,f: spec_fn(int,int)->int)
    requires n>0,
    ensures sum(m*n,|p: int|f(p%(n as int),p/(n as int)))==sum(m,|y: int|sum(n,|x: int|f(x,y))),
{
    let source=|p: int|f(p%(n as int),p/(n as int));
    sum_blocks(m,n,source);
    let g=|y: int|sum(n,|x: int|source(y*n+x));
    let h=|y: int|sum(n,|x: int|f(x,y));
    assert forall|y: int| 0<=y<m implies #[trigger] g(y)==#[trigger] h(y) by {
        assert forall|x: int| 0<=x<n implies #[trigger] source(y*n+x)==#[trigger] f(x,y) by {
            vstd::arithmetic::div_mod::lemma_div_multiples_vanish_fancy(y,x,n as int);
            vstd::arithmetic::div_mod::lemma_mod_multiples_vanish(y,x,n as int);
            vstd::arithmetic::div_mod::lemma_small_mod(x as nat,n);
            assert(y*n==n*y) by(nonlinear_arith);
        }
        sum_equal(n,|x: int|source(y*n+x),|x: int|f(x,y));
    }
    sum_equal(m,g,h);
}

pub(crate) proof fn sum_credit_bound(n: nat,w: spec_fn(int)->int,b: spec_fn(int)->int,
    g: spec_fn(int)->int,p: spec_fn(int)->int)
    requires forall|t: int| 0<=t<n ==> #[trigger] w(t)<=#[trigger] b(t)+#[trigger] g(t)
        +#[trigger] p(t-1)-#[trigger] p(t),
    ensures sum(n,w)<=sum(n,b)+sum(n,g)+p(-1)-p(n-1),
    decreases n,
{
    if n>0 { sum_credit_bound((n-1) as nat,w,b,g,p); }
}

pub(crate) proof fn sum_order(n: nat,f: spec_fn(int)->int,g: spec_fn(int)->int)
    requires forall|i: int| 0<=i<n ==> #[trigger] f(i)<=#[trigger] g(i),
    ensures sum(n,f)<=sum(n,g),
    decreases n,
{
    if n>0 { sum_order((n-1) as nat,f,g); }
}

pub(crate) proof fn opposite_grid(n: nat,f: spec_fn(int,int)->int,g: spec_fn(int,int)->int)
    requires n>0,
    ensures sum(n*n,|p: int|f(p%(n as int),p/(n as int))+g(p/(n as int),p%(n as int)))
        ==sum(n,|line: int|sum(n,|t: int|f(line,t)))+sum(n,|line: int|sum(n,|t: int|g(line,t))),
{
    grid_sum(n,n,f);
    sum_swap(n,n,f);
    let transpose=|x: int,y: int|g(y,x);
    grid_sum(n,n,transpose);
    let a=|p: int|f(p%(n as int),p/(n as int));
    let b=|p: int|g(p/(n as int),p%(n as int));
    sum_add(n*n,a,b);
    sum_equal(n*n,|p: int|a(p)+b(p),
        |p: int|f(p%(n as int),p/(n as int))+g(p/(n as int),p%(n as int)));
    sum_equal(n*n,|p: int|transpose(p%(n as int),p/(n as int)),b);
    let h=|line: int|sum(n,|t: int|transpose(t,line));
    let k=|line: int|sum(n,|t: int|g(line,t));
    assert forall|line: int| 0<=line<n implies #[trigger] h(line)==#[trigger] k(line) by {
        sum_equal(n,|t: int|transpose(t,line),|t: int|g(line,t));
    }
    sum_equal(n,h,k);
}

pub(crate) proof fn range_count(n: nat,p: spec_fn(int)->bool)
    ensures Set::<int>::range(0,n as int).filter(p).len() as int==sum(n,|i: int|bit(p(i))),
    decreases n,
{
    broadcast use vstd::set_lib::group_set_properties;
    if n>0 {
        range_count((n-1) as nat,p);
        let all=Set::<int>::range(0,n as int).filter(p);
        let rest=Set::<int>::range(0,n-1).filter(p);
        if p(n-1) {assert(all =~= rest.insert(n-1));}
        else {assert(all =~= rest);}
    } else {
        assert(Set::<int>::range(0,n as int).filter(p) =~= Set::<int>::empty());
    }
}

pub(crate) proof fn sum_signed_pair(n: nat,w: spec_fn(int)->int,g: spec_fn(int)->int,
    b: spec_fn(int)->int,p: spec_fn(int)->int)
    requires forall|t: int| 0<=t<n ==> #[trigger] w(t)==2*#[trigger] g(t)-2*#[trigger] b(t)
        +#[trigger] p(t)-#[trigger] p(t-1),
    ensures sum(n,w)==2*sum(n,g)-2*sum(n,b)+p(n-1)-p(-1),
    decreases n,
{
    if n>0 {sum_signed_pair((n-1) as nat,w,g,b,p);}
}

pub(crate) proof fn sum_shift(n: nat,f: spec_fn(int)->int)
    ensures sum(n,|i: int|f(i-1))==sum(n,f)+f(-1)-f(n-1),
    decreases n,
{
    if n>0 {sum_shift((n-1) as nat,f);}
}

pub(crate) proof fn sum_two_identity(n: nat,w: spec_fn(int)->int,c: spec_fn(int)->int,s: spec_fn(int)->int)
    requires forall|i: int| 0<=i<n ==> #[trigger] w(i)==#[trigger] c(i)+2*#[trigger] s(i),
    ensures sum(n,w)==sum(n,c)+2*sum(n,s),
    decreases n,
{
    if n>0 {sum_two_identity((n-1) as nat,w,c,s);}
}

pub(crate) proof fn sum_double_difference(n: nat,w: spec_fn(int)->int,g: spec_fn(int)->int,b: spec_fn(int)->int)
    requires forall|i: int| 0<=i<n ==> #[trigger] w(i)==2*#[trigger] g(i)-2*#[trigger] b(i),
    ensures sum(n,w)==2*sum(n,g)-2*sum(n,b),
    decreases n,
{
    if n>0 {sum_double_difference((n-1) as nat,w,g,b);}
}

pub(crate) proof fn sum_interval(n: nat,a: int,b: int)
    requires 0<=a<=b,
    ensures sum(n,|i: int|bit(a<=i<b))==if n<=a {0}else if n>=b {b-a}else{n-a},
    decreases n,
{
    if n>0 {sum_interval((n-1) as nat,a,b);}
}

pub(crate) proof fn sum_add_sub(n: nat,w: spec_fn(int)->int,a: spec_fn(int)->int,
    b: spec_fn(int)->int,c: spec_fn(int)->int)
    requires forall|i: int| 0<=i<n ==> #[trigger] w(i)==#[trigger] a(i)+#[trigger] b(i)-#[trigger] c(i),
    ensures sum(n,w)==sum(n,a)+sum(n,b)-sum(n,c),
    decreases n,
{
    if n>0 {sum_add_sub((n-1) as nat,w,a,b,c);}
}

pub(crate) proof fn sum_difference(n: nat,w: spec_fn(int)->int,a: spec_fn(int)->int,b: spec_fn(int)->int)
    requires forall|i: int| 0<=i<n ==> #[trigger] w(i)==#[trigger] a(i)-#[trigger] b(i),
    ensures sum(n,w)==sum(n,a)-sum(n,b),
    decreases n,
{
    if n>0 {sum_difference((n-1) as nat,w,a,b);}
}

}
