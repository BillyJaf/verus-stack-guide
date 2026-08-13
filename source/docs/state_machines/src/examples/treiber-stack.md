# Lock-Free Stack

Now let's have a go at verifying a a lock-free stack - a [Treiber Stack](https://en.wikipedia.org/wiki/Treiber_stack). 

A Treiber Stack, is a stack data structure that permits concurrency through atomics, not locks. Of course, this property alone does not imply that the stack it is [lock-free](https://en.wikipedia.org/wiki/Non-blocking_algorithm) in the formal sense. There are three main categories of progress guarantees that concurrent programs can satisfy, with the Treiber Stack falling under the second:

 1. Blocking / Lock-Based
 2. Lock-Free
 3. Wait-Free

System wide progress is guaranteed; if a thread fails to make progress, it is the direct result of another thread making progress. This is a property that is not seen in Lock-Based algorithms if, for example, a thread is descheduled while holding a mutex. With the formalities out of the way, let's take a look at what operations are present in a Treiber Stack, and how we can implement & verify them!

## Operations:

The Treiber Stack at its core is still just a stack. As a result, the two main operations that we must support are `push` and `pop`. The stack is made using a linked-list structure where each element maintains a pointer to the one below it in the stack. The actual stack object that we will interact therefore only needs to hold the address of the top element in the stack, and we can `push` and `pop` by updating this top address. In this explanation, I will use the words 'top' & 'head' interchangeably, and also the words 'base' & 'bottom' interchangeably.

Consider the following basic setup for this stack that stores `u32` elements:
```rust
pub struct Stack {
    pub top_addr: AtomicUsize,
}

pub struct StackCell {
    pub elem: u32,
    pub next_addr: usize,
}
```

The following is an overview of how `push(x)` works:
 1. Atomically load the address of the top element - `top_addr`.
 2. Construct a new `StackCell` with elem `x` and next address `top_addr`.
 3. Compare the address of the stack's current `top_addr` with the previously loaded `top_addr`.
 4. If the address has not changed, then no other thread has changed the stack - atomically set the stacks `top_addr` to the address of your newly constructed `StackCell` and return.
 5. If the stack's `top_addr` has changed, then another thread has changed the stack. Repeat from step 1.

The following is an overview of how `pop` works:
 1. Atomically load the address of the top element - `top_addr`.
 2. If `top_addr` is `null`, then there is nothing to pop; return.
 3. Otherwise, there is an element to pop. Cast the address to a stack cell pointer `*const StackCell` and derefence it to obtain the `elem` and `next_addr`.
 4. Compare the address of the stack's current `top_addr` with the previously loaded `top_addr`.
 5. If the address has not changed, then no other thread has changed the stack - atomically set the stacks `top_addr` to the address you previously obtained, `next_addr`, and return the popped element `elem`.
 6. If the stack's `top_addr` has changed, then another thread has changed the stack. Repeat from step 1.

### The ABA Problem:

Those with a keen eye might have noticed a problem with the above algorithm known as the [ABA problem](https://en.wikipedia.org/wiki/ABA_problem). The crux of the problem is as follows: we assume that the stack has not been interacted with by comparing addresses, but there is nothing stopping everything underneath this address from changing. To illustrate this point, consider the following stack:

```rust
top_addr = A
*(A as *const StackCell).next_addr = B
*(B as *const StackCell).next_addr = C
```

Imagine that there are two threads interacting with the stack `T1` and `T2`. The first thread `T1` beings a `pop` operation: it loads the `top_addr = A` and dereferences it as a `StackCell` to get the `next_addr = B`. Before it continues, it is preempted and the second thread `T2` begins execution. `T2` successfully pops the top two elements from the stack, which now just consists of:
```
top_addr = C
```
We free the memory that is associated with `A` and `B` as they have been popped. `T2` then pushes a new element to the stack, however, because we have freed `A` & `B`'s memory, the new `StackCell` gets created at address `A`. The push proceeds and we get:
```rust
top_addr = A
*(A as *const StackCell).next_addr = C
```
`T2` returns and allows `T1` to continue execution. But look at what has happened! `T1` compares the address it previously loaded, `A`, with the current `top_addr`, which is also A - it's a match. `T1` assumes that there has been no change and atomically set the stacks `top_addr` to the address it previously obtained, `B`. This is a huge problem, `B` is no longer a valid address! The stack now looks something like this:
```rust
top_addr = B
*(B as *const StackCell).next_addr = ??? // Undefined behaviour as B is deallocated
```
It is clear from this that our stack is now disjoint. `C`, and anything that was in the stack below `C`, is now 'lost' - the `top_addr` of the stack no longer points to a valid `StackCell`.

So how can we solve this problem?

The solution is actually somewhat trivial, but also undesireable from a performance perspective - we let `StackCell`s leak into the heap. If we never deallocate `StackCell`s, then we can never reuse their memory, and not run into this problem. Of course, as I mentioned, this is not desireable from a performance perspective - but this guide book is more focused on verification rather than performance!

## Implementation:

### Tokenised State Machine Fields:

So let's have a go at an implementation and an explanation as to why tokenised state machines are perfect for this data structure.

Firstly, looking at the above pseudo-code, you'll notice that there are several times where we did something like this while popping elements:
```rust
*(A as *const StackCell).elem
*(A as *const StackCell).next_addr
```
You might notice that this is `unsafe`, and as a result this dereference must occur within an `unsafe` block within normal Rust - not good! Luckily, Verus has the perfect tool for bypassing this problem: [Permissioned Pointers](https://verus-lang.github.io/verus/verusdoc/vstd/simple_pptr/struct.PPtr.html).

From the source, a `PPtr<V>` is a *wrapper around a raw pointer to a heap-allocated `V`* - in our case, we will be using a `PPtr<StackCell>`. Essentially, when we create a `PPtr`, we are given a pointer and a permission. To read the underlying memory of the pointer, we must have a reference to the permission, and to write to the underlying memory we need a mutable reference to the permission. This is perfect for our use case. You'll notice that in both the `push` and `pop` algorithms, we never need to mutate any underlying data of a `StackCell`. When we create a `StackCell`, it exists as a constant until the process terminates - the only data that is mutated is the `AtomicUsize` that houses the `top_addr` (which we'll discuss soon). To show you what the start of the `push` operation looks like, we do the following (where `elem` is the element we are pushing):

```rust
// Construct a new StackCell by loading the top address
let new_stack_cell = StackCell { elem, next: self.top_addr.load() };

// Wrap the new StackCell in a PPtr
// permission_guarded_new_stack_cell: PPtr<StackCell>
// new_stack_cell_permission: PointsTo<StackCell>
let (permission_guarded_new_stack_cell, Tracked(new_stack_cell_permission)) = PPtr::new(
    new_stack_cell,
);
```

So, what you need to remember is that instead of dereferencing raw pointers, we can create a `StackCell` and wrap it in a `PPtr`. Then, to read from the `StackCell`, we only need a reference to the related permission `PointsTo<StackCell>`. As we only need a reference, these permissions can exist in a shared data structure that all threads can access. Once again, Verus saves the day! This is where it becomes clear why a tokenised state machine is the perfect companion for our implementation.

Looking through the guide, we find the [storage_map](https://verus-lang.github.io/verus/state_machines/strategy-storage-map.html) sharding strategy which even says that the tokens stored are typically of type `PointsTo<V>`. Essentially, we can shard a field in our tokenised state machine with the `storage_map` strategy, and deposit our permissions (`PointsTo<StackCell>`) in this field. Then, when we need a reference to a permission so that we can read a `PPtr`'s underlying memory, we can use the `guard` command on the field to obtain a reference to the relevant `PointsTo<StackCell>`. To put this all together, our current naive tokenised state machine might look something like this:

```rust
type StackCellAddress = usize;

tokenized_state_machine!{
    machine {
        fields {
            #[sharding(storage_map)]
            pub permissions: Map<StackCellAddress, PointsTo<StackCell>>,
        }
    }
}
```

You'll notice that we've used a `StackCellAddress` as the key to the map. This is somewhat redundant as `PointsTo<V>` has a method `addr` that returns the address that the permission is for, but every `Map` entry requires a key, so this will do. We can tie this back in later by having an invariant asserting that every permission's address and map key are equal. Then, when a `push` occurs, we can update the `permissions` field in our TSM with the new permission `PointsTo<StackCell>`. Other threads can `guard` this token through the TSM to obtain a reference to the permission and read the underlying memory, neat!

Let's have a think about what other fields we would need in our TSM. Having another look through the [storage_map](https://verus-lang.github.io/verus/state_machines/strategy-storage-map.html) documentation at the `guard` command, we can notice something interesting. The command syntactically looks like:
```rust
guard field >= [ k => tok ];
```

Which in our case looks like:
```rust
// permission: PointsTo<StackCell>
guard field >= [ StackCellAddress => permission ];
```

Which in a transition has meaning:
```rust
// permission: PointsTo<StackCell>
assert field.dom().contains(StackCellAddress) && field[StackCellAddress] == permission;
```
From this, you might ask yourself "if we already deposited the permission, then how can we pass it as an argument to the guard?" - great question. It is clear that we will need some way of keeping track of what permissions are stored within our map so that we may retrieve references to them. We can use a pattern that can be seen used in various other proofs (for example [here](https://github.com/verus-lang/verus/blob/92f466f247f45128c630d1c843fd6e27d2115587/examples/state_machines/maps.rs)). In our TSM, we can add another map-like field that is identical to our storage map, with the caveat that this field is purely ghost - it doesn't hold physical values and the permissions can't be used in exec code. The benefit of this, is that every entry in this map, while it **cannot** directly be used to access memory behind a `PPtr`, it **can** be used as a witness token for our `storage_map`. For example:

```rust
type StackCellAddress = usize;

tokenized_state_machine!{
    machine {
        fields {
            #[sharding(persistent_map)]
            pub witnesses: Map<StackCellAddress, PointsTo<StackCell>>,

            #[sharding(storage_map)]
            pub permissions: Map<StackCellAddress, PointsTo<StackCell>>,
        }
    }

    #[invariant]
    pub fn witnesses_reflect_permissions_inv(&self) -> bool {
        self.witnesses == self.permissions
    }
}
```

You'll notice that we are sharding this `witness` field with the [persistent_map](https://verus-lang.github.io/verus/state_machines/strategy-persistent-map.html) strategy. The reason for this are twofold:
 1. The tokens implement `Copy` - there can be any such number of tokens for a key and hence we may distribute witnesses to many threads at once.
 2. Since we are allowing `PPtr<StackCell>`s to leak into the heap, we never need to revoke a witness. The `persistent_map` strategy has no `remove` method.

Great, so we can store the real permissions for each `PPtr<StackCell>` in the `permissions` map, and store a ghost witness in the `witnesses` map. Since we may distribute these ghost witnesses however we like, each thread can use any ghost witness to `guard`, and therefore gain a reference to, any real permission with something like this:

```rust
property!{
    get_permission_reference(stack_cell_address: StackCellAddress, stack_cell_permission: PointsTo<StackCell>) {
        have witnesses >= [stack_cell_address => stack_cell_permission];
        guard permissions >= [stack_cell_address => stack_cell_permission];
    }
}
```
This all hinges on the invariant that we stated in the TSM above, Verus knows that the maps are equal and therefore it knows that if we have a `(StackCellAddress, PointsTo<StackCell>)` pair in our `witnesses` map, then there must be an equal pair in the `permissions` map. The only difference is that the `permissions` map houses the exec value. So what else do we need in our TSM?

You might notice that, so far, all of our fields are sharded with map strategies. Since we require both of these maps to be equal, we know that when we want to insert a key-value pair, we must demonstrate the more binding requirement out of the two. i.e. to insert a key `k` into these maps, we must show:
```rust
!permissions.dom().contains(k);
```

But there is one problem with this, since the `permissions` field is sharded with the `storage_map` strategy, attempting to `require` this fact in a transition results in an error. The following is an example transition that reflects how we might try to show that a key is not in the map:

```rust
transition!{
    empty_domain(key: StackCellAddress)
    {
        require(!pre.permissions.dom().contains(key));
    }
}


error: A 'storage_map' field cannot be directly referenced here
   --> ..\stack.rs:XXX:XX
    |
XXX |  require(!pre.permissions.dom().contains(key));
    |  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

So, if we can't refer to a `storage_map` field, then how else might we demonstrate that the `permissions` field doesn't contain an address so that we may `deposit` a permission? We need another field!

> **Note:**
>
> This is true at the time of writing. The `storage_map` section of the guide specifically notes that: *The deposit instruction has an inherent safety condition that key is not already present in the pre-state. (Note: this is not strictly necessary, and the restriction may be removed later.)*
>
> Therefore, this field we are about to include might no longer be necessary in the future, we urge the reader to check the documentation for more details!

How will another field solve our problems? Well, we can have a field with type `set` and shard it with the [variable](https://verus-lang.github.io/verus/state_machines/strategy-variable.html) strategy. Then, we may have an invariant that this set is equal to the domain of our maps. Now, when we want to insert into the maps, we only need to `require` that the key is not within the set - a legal expression! Here is an example of what that field, invariant and transition might look like:

```rust
type StackCellAddress = usize;

tokenized_state_machine!{
    machine {
        fields {
            #[sharding(variable)]
            pub addresses: Set<StackCellAddress>,

            #[sharding(persistent_map)]
            pub witnesses: Map<StackCellAddress, PointsTo<StackCell>>,

            #[sharding(storage_map)]
            pub permissions: Map<StackCellAddress, PointsTo<StackCell>>,
        }
    }

    #[invariant]
    pub fn witnesses_reflect_permissions_inv(&self) -> bool {
        self.witnesses == self.permissions
    }

    #[invariant]
    pub fn addresses_reflect_permissions_inv(&self) -> bool {
        self.permissions.dom() == self.addresses
    }

    transition!{
        push(permission: PointsTo<StackCell>)
        {
            require(!pre.addresses.contains(permission.addr()));

            update addresses = pre.addresses.insert(permission.addr());
            deposit permissions += [permission.addr() => permission];
            add witnesses (union)= [permission.addr() => permission];

        }
    }
}
```
Despite the fact that our program will be multithreaded, it is fine to use a field sharded with the `variable` strategy - you'll see that we will only be interacting with this field through atomics. 

On top of this, it is also required in our TSM to have a field that reflects our real `exec` stack. If this field represents our stack, and we only ever interact with this field through stack operations (push and pop) within transitions, then we can be sure that our implementation is functionally correct.

> **Note:**
>
> This is not entirely true, but for the simplicity of the guide it is sufficient. For a more rigorous proof, the user would also have to include a linearised history of events and return a witness token to the user after every operation. The purpose of the witness token is to provide proof to the user that what they thought happend, actually did. It might be confusing, but this witness token is different to the kind we have already been using...
>
> Essentially, without a linearised history, the program could technically just do nothing when a `push` or a `pop` is called, and this would satisfy the specifications provided that the intial state is valid. Adding these new witness tokens demonstrates that something has actually occurred, but adding them is too complicated for this guide.
>
> Unless you want to be very rigorous, you can forget this ever happened!

So, to add those fields to our TSM, we might include fields like this:


```rust
type StackCellAddress = usize;

#[sharding(variable)]
pub current_stack_addresses: Seq<StackCellAddress>,

#[sharding(variable)]
pub popped_addresses: Set<StackCellAddress>,
```

And have them interact with the field that maintains all addresses that have housed a `StackCell` in our stack like so:

```rust
type StackCellAddress = usize;

#[sharding(variable)]
pub current_stack_addresses: Seq<StackCellAddress>,

#[sharding(variable)]
pub popped_addresses: Set<StackCellAddress>,

#[sharding(variable)]
pub addresses: Set<StackCellAddress>,

#[invariant]
pub fn current_stack_union_popped_inv(&self) -> bool {
    self.current_stack_addresses.to_set().union(self.popped_addresses) == self.addresses
}
```

Now is a good time to mention that, in a typical Treiber Stack, you might determine if the stack is empty depending on if the address of the top `StackCell` is `null`. However, we will be using the addresses of `PPtr`s, and there is no way to get a `null` `PPtr`. Fortunately, there is an easy work around - unfortunately, we will need to add another field to our TSM.

We can make use of the `PPtr::<V>::empty()` method, which *allocates heap memory for type `V`, leaving it uninitialized*. Instead of having a `null` address, we can instead store a field in the TSM to represent the base of the stack and initialise it with a `PPtr::<StackCell>::empty()`. Then, whenver we check if the stack is empty, we can instead check if the top address is the base address.

Thats all we need for our TSM `fields`! Of course, we will have to add more invariants, transitions, properties, etc. But it would be good to first show you all the fields we talked about:
```rust
type StackCellAddress = usize;

tokenized_state_machine!{
    machine {
        fields {
            // Book Keeping

            #[sharding(constant)]
            pub base_address: StackCellAddress,

            // Stack Representation

            #[sharding(variable)]
            pub current_stack_addresses: Seq<StackCellAddress>,

            #[sharding(variable)]
            pub popped_addresses: Set<StackCellAddress>,

            // Witnesses and Permissions

            #[sharding(variable)]
            pub addresses: Set<StackCellAddress>,

            #[sharding(persistent_map)]
            pub witnesses: Map<StackCellAddress, PointsTo<StackCell>>,

            #[sharding(storage_map)]
            pub permissions: Map<StackCellAddress, PointsTo<StackCell>>,
        }
    }    
}
```

### Tokenised State Machine Invariants:

Now that we have our fields, let's have a go at writing some invariants that should hold throughout all transitions we write.

> **Note:**
>
> This is probably not the best method of developing if you are trying to do something from scratch. For example, while making this stack, I did not first decide what fields would be useful and then the invariants on them. The actual methodology I used was:
>
> Have a vague idea of the TSM structure. Implement that structure. Write some invariants. Write some transitions. Everything breaks. Try again.
>
> However, for the purpose of the guide, its best to tackle things section by section.

So what invariants do we need on the stack? Well, we already had a few that we discussed earlier:

```rust
#[invariant]
pub fn witnesses_reflect_permissions_inv(&self) -> bool {
    self.witnesses == self.permissions
}

#[invariant]
pub fn addresses_reflect_permissions_inv(&self) -> bool {
    self.permissions.dom() == self.addresses
}

#[invariant]
pub fn current_stack_union_popped_inv(&self) -> bool {
    self.current_stack_addresses.to_set().union(self.popped_addresses) == self.addresses
}
```

Remember, our TSM holds a representation the real `exec` stack - our invariants should accomodate this and assert things that uphold this structure. Given this, we can also add the following invariants with very little explanation:

```rust
// current_stack_addresses: Seq<StackCell>
// It could technically have duplicate addresses, so we must prohibit this:
#[invariant]
pub fn no_duplicates_inv(&self) -> bool {
    self.current_stack_addresses.no_duplicates()
}

// As we are not reusing `PPtr`s and instead allowing them to leak, once a
// StackCell is popped, it can never return to the stack
#[invariant]
pub fn current_stack_disjoint_popped_inv(&self) -> bool {
    self.current_stack_addresses.to_set().disjoint(self.popped_addresses)
}

// The union of the addresses in our current stack and the set of popped
// addresses should equal the set of all addresses included in the stack
#[invariant]
pub fn current_stack_union_popped_inv(&self) -> bool {
    self.current_stack_addresses.to_set().union(self.popped_addresses) == self.addresses
}

// The base address is always present at the base of the stack
#[invariant]
pub fn current_stack_contains_base_address_inv(&self) -> bool {
    &&& self.current_stack_addresses.contains(self.base_address)
    &&& self.current_stack_addresses.first() == self.base_address
}

// The base address has a witness - this invariant will become somewhat redundant, 
// but it doesn't hurt to include it.
#[invariant]
pub fn base_address_witness_exists_inv(&self) -> bool {
    self.witnesses.dom().contains(self.base_address)
}
```

Great, we already have a fair few invariants - let's have a think about what else we need to include. Recall that we are using a `storage_map` to keep the `permissions`, and a `persistent_map` to keep the `witnesses`. It was mentioned earlier that the keys in each map can just be the associated address of the value. That is to say:

```rust
#[invariant]
pub fn maps_are_correct_inv(&self) -> bool {
    forall |addr: StackCellAddress| #![auto]
        (
            self.witnesses.dom().contains(addr) ==>
                self.witnesses.index(addr).addr() == addr
        ) && (
            self.permissions.dom().contains(addr) ==>
                self.permissions.index(addr).addr() == addr
        )
}
```