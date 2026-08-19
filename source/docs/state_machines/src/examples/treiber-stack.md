# Lock-Free Stack

Now let's have a go at verifying a lock-free stack; a [Treiber Stack](https://en.wikipedia.org/wiki/Treiber_stack). 

A Treiber Stack is a stack data structure that permits concurrency through atomics, not locks. Of course, this property alone does not imply that the stack it is [lock-free](https://en.wikipedia.org/wiki/Non-blocking_algorithm) in the formal sense. There are three main categories of progress guarantees that concurrent programs can satisfy, with the Treiber Stack falling under the second:

 1. Blocking / Lock-Based
 2. Lock-Free
 3. Wait-Free

In a Treiber Stack, system wide progress is guaranteed; if a thread fails to make progress, it is the direct result of another thread making progress. This is a property that is not seen in Lock-Based algorithms if, for example, a thread is descheduled while holding a mutex. With the formalities out of the way, let's take a look at what operations are present in a Treiber Stack, and how we can implement & verify them!

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
 2. Construct a new `StackCell` with `elem = x` and `next_addr = top_addr`.
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

Imagine that there are two threads, `T1` and `T2` interacting with the stack. The first thread `T1` beings a `pop` operation: it loads the `top_addr = A` and dereferences it as a `StackCell` to get the `next_addr = B`. Before it continues, it is preempted and the second thread `T2` begins execution. `T2` successfully pops the top two elements from the stack so that it then it only consists of:
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

### Tokenized State Machine Fields:

So let's have a go at an implementation and an explanation as to why tokenized state machines are perfect for this data structure.

Firstly, looking at the above pseudo-code, you'll notice that there are several times where we did something like this while popping elements:
```rust
*(A as *const StackCell).elem
*(A as *const StackCell).next_addr
```
You might notice that this is `unsafe`, and as a result this dereference must occur within an `unsafe` block within normal Rust - not good! Luckily, Verus has the perfect tool for bypassing this problem: [Permissioned Pointers](https://verus-lang.github.io/verus/verusdoc/vstd/simple_pptr/struct.PPtr.html).

From the source, a `PPtr<V>` is a *wrapper around a raw pointer to a heap-allocated `V`* - in our case, we will be using a `PPtr<StackCell>`. Essentially, when we create a `PPtr<V>`, we are given an `exec` pointer and a `Tracked` permission. To read the underlying memory of the pointer, we must have a `Tracked` reference to the permission, and to write to the underlying memory we need a `Tracked` mutable reference to the permission. The reason that the permission is wrapped in `Tracked` throughout all of this is because it is a `tracked` ghost object - it is linear and erased at compile time. Wrapping a proof object with `Tracked` allows it to be passed as an argument to a function (or returned from one) while still having the proof object erased at compile time, leaving only a zero-sized `Tracked` wrapper. No runtime penalty required, neat! The permissioned pointer is perfect for our use case. 

You'll notice that in both the `push` and `pop` algorithms, we never need to mutate any underlying data of a `StackCell`. When we create a `StackCell`, it exists as a constant until the process terminates - the only data that is mutated is the `AtomicUsize` that houses the `top_addr` (which we'll discuss soon). To show you what the start of the `push` operation looks like, and how we can use permissioned pointers, we do the following (where `elem` is the element we are pushing):

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

So, what you need to remember is that instead of dereferencing raw pointers, we can create a permissioned pointer that points to our `StackCell`. Then, to read from the underlying data of the `PPtr<StackCell>`, we only need a reference to the related permission `PointsTo<StackCell>`. As we only need a reference, these permissions can exist in a shared data structure that all threads can access. Once again, Verus saves the day! This is where it becomes clear why a tokenized state machine is the perfect companion for our implementation.

Looking through the guide, we find the [storage_map](https://verus-lang.github.io/verus/state_machines/strategy-storage-map.html) sharding strategy which even says that the tokens stored are typically of type `PointsTo<V>`. Essentially, we can shard a field in our tokenized state machine with the `storage_map` strategy, and deposit our permissions in this field. Then, when we want to read the data from a `PPtr<StackCell>`, we can use the `guard` command on the field to obtain a reference to the relevant `PointsTo<StackCell>`. To visualise this field, our current tokenized state machine might look something like this:

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

You'll notice that we've used a `StackCellAddress` as the key to the map. This is somewhat redundant as `PointsTo<V>` has a method `addr` that returns the address that the permission is for, but every `Map` entry requires a key, so this will do. We can tie this back in later by having an invariant asserting that every permission's address and map key are equal. When a `push` occurs, we can update the `permissions` field in our TSM to include the new permission `PointsTo<StackCell>`. Other threads can `guard` this token through the TSM to obtain a reference to the permission and read the underlying memory, neat!

Let's have a think about what other fields we would need in our TSM. Having another look through the [storage_map](https://verus-lang.github.io/verus/state_machines/strategy-storage-map.html) documentation at the `guard` command, we can notice something interesting. The command syntactically looks like:
```rust
guard field >= [ k => tok ];
```

Which in our case looks like:
```rust
// permission: PointsTo<StackCell>
guard field >= [ StackCellAddress => permission ];
```

And in a transition has meaning:
```rust
// permission: PointsTo<StackCell>
assert field.dom().contains(StackCellAddress) && field[StackCellAddress] == permission;
```
It is clear that we will need some way to keep track of what permissions are stored within our map so that we may retrieve references to them. From this, you might ask yourself "if we already deposited the permission, and the permission is linear, then how can we pass it as an argument to the guard?" - great question. We can use a pattern that can be seen used in various other proofs (for example [here](https://github.com/verus-lang/verus/blob/92f466f247f45128c630d1c843fd6e27d2115587/examples/state_machines/maps.rs)). 

In our TSM, we can add another map-like field that is identical to our storage map, with the caveat that this field holds `Ghost` copies of our `tracked` tokens. Since `Ghost` types are erased at compile time and can freely 'copy' any value (even from things that do not implement `Copy`), they fit our requirements perfectly. If we can have another field that is identical to our `storage_map` value wise, but is not `tracked`, then we may use those tokens as witnesses for the `tracked` tokens. This is what that looks like in practice:

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

Firstly, you'll notice that we are sharding this `witness` field with the [persistent_map](https://verus-lang.github.io/verus/state_machines/strategy-persistent-map.html) strategy. The reason for this are twofold:
 1. When inserting a key-value pair to the map, a fresh token that implements `Copy` is created. Hence, there can be any such number of tokens for a key and hence we may distribute witnesses to many threads at once.

 2. Since we are allowing `PPtr<StackCell>`s to leak into the heap, we never need to revoke a witness. The `persistent_map` strategy has no `remove` method.

Secondly, you'll notice that we are asserting the two fields are always equal to each other. This way, we can be sure that whenever we have one of these witness tokens, while we may not be able use it to directly to read from a `PPtr<StackCell>`, we can use it to `guard` a `tracked` permission and use *that* permission to read the memory. In practice, that property looks like this:


```rust
property!{
    get_permission_reference(stack_cell_address: StackCellAddress, stack_cell_permission: PointsTo<StackCell>) {
        have witnesses >= [stack_cell_address => stack_cell_permission];
        guard permissions >= [stack_cell_address => stack_cell_permission];
    }
}
```
This all hinges on the invariant that we stated in the TSM above, Verus knows that the maps are equal and therefore it knows that if we have a `(StackCellAddress, PointsTo<StackCell>)` pair in our `witnesses` map, then there must be an equal pair in the `permissions` map. The only difference is that the `permissions` map "stores" the `tracked` value that is useable. So what else do we need in our TSM?

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
> This is not entirely true, but for the simplicity of the guide it is sufficient. For a more rigorous proof, the user would also have to include a linearised history of events and return a witness token to the user after every operation. The purpose of the witness token is to give proof to the user that their intended action actually happened. It might be confusing, but this witness token is different to the kind we have already been using...
>
> Essentially, without a linearised history, the program could technically just do nothing when a `push` or a `pop` is called, and this would satisfy all specifications provided (provided that the intial state satisfies them). Adding these new witness tokens demonstrates that something has actually occurred, but adding them is too complicated for this guide.
>
> Unless you want to be very rigorous, you can forget that I sai anything!

So, to add those stack-representation fields to our TSM, we might include fields like this:


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

Now is a good time to mention that, in a typical Treiber Stack, you might determine if the stack is empty depending on if the address of the top `StackCell` is `null`. However, we are using `PPtr`s to house our `StackCell`s, and there is no way to get a `null` `PPtr`. Fortunately, there is an easy work around - unfortunately, we will need to add another field to our TSM.

We can make use of the `PPtr::<V>::empty()` method, which *allocates heap memory for type `V`, leaving it uninitialized*. Instead of having a `null` address, we can instead store a field in the TSM to represent the base of the stack and initialise it with a `PPtr::<StackCell>::empty()`. Then, whenver we check if the stack is empty, we can instead check if the top address is the base address.

And thats the last field we need for our TSM! Of course, we will have to add more invariants, transitions, properties, etc. But it would be good to first show you all the fields we talked about:
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

### Tokenized State Machine Invariants:

Now that we have our fields, let's have a go at writing some invariants that should hold throughout all transitions we write.

> **Note:**
>
> This is probably not the best method of developing if you are trying to do something from scratch. For example, while making this stack, I did not first decide what fields would be useful and then the invariants on them. The actual methodology I used was:
> - Have a vague idea of the TSM structure. 
> - Implement that structure. 
> - Write some invariants. 
> - Write some transitions. 
> - Everything breaks, try again.
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

Remember, our TSM holds a representation of the real `exec` stack - our invariants should accomodate this and assert things that uphold this structure. Given this, we can also add the following invariants with very little explanation:

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

// Everything in our current stack representation has a witness.
// In other words, if our current stack representation was a set, then it is a subset
// of the witnesses domain.
#[invariant]
pub fn current_stack_has_witnesses_inv(&self) -> bool {
    self.current_stack_addresses.to_set().subset_of(self.witnesses.dom())
}
```

Great, we already have a fair few invariants - let's have a think about what else we need to include. Recall that we are using a `storage_map` to keep the `permissions`, and a `persistent_map` to keep the `witnesses`. It was mentioned earlier that the key of each key-value pair can be the associated address of the value. That is to say:

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

Also, remember that we are not using a `null` address to signal that the stack is empty, but instead we are maintaining a specific unitialised `PPtr` with address `base_address`. Apart from this `base_address` all other permissions in our TSM should be initialised, hence:

```rust
#[invariant]
pub fn permissions_are_init_except_base_inv(&self) -> bool {
    forall |addr: StackCellAddress| #![auto]
        self.permissions.dom().contains(addr) ==> (
            addr != self.base_address <==> self.permissions.index(addr).is_init()
        )
}
```

The last invariant we need is what ties it all together. I recommend reading the invariant first and then the explanation - try to see if you can work out what it means and why it might be necessary:

```rust
#[invariant]
pub fn witnesses_contains_next_witness_inv(&self) -> bool {
    forall |addr: StackCellAddress| #![auto]
        (
            self.witnesses.dom().contains(addr) &&
            addr != self.base_address
        ) ==>
        self.witnesses.dom().contains(
            self.witnesses.index(addr).value().next
        )
}
```

Recall that the goal of our invariants is to uphold the stack properties and to store the relevant `PPtr` permissions. We know that a `PointsTo<StackCell>` permission can only be stored in our TSM if the relevant `PPtr<StackCell>` was pushed to the stack. But, a `PPtr<StackCell>` can only be pushed to the stack (excluding the base) if it points to a valid `next` node. i.e. the `next` field is the address of a `PPtr<StackCell>` that was already pushed to the stack. To put it simply: if a `PPtr<StackCell>` is on the top of the stack, then it must point to a previously pushed `PPtr<StackCell>`. Hence, if there is a witness token for a `PPtr<StackCell>`, then there must also be a witness token for the `PPtr<StackCell>` that it points to.

And with that, we have all of the invariants that we need in our TSM - hopefully none of the stated invariants should come as a surprise to the reader.

### Tokenized State Machine Transitions:

Now that we have the invariants, we can write the transitions in our TSM. Starting off with the `init` initialiser that doesn't really need explanation:

```rust
init!{
    initialize(base_permission: PointsTo<StackCell>)
    {
        require(base_permission.is_uninit());
        init base_address = base_permission.addr();
        init current_stack_addresses = Seq::empty().push(base_permission.addr());
        init popped_addresses = Set::empty();
        init addresses = Set::empty().insert(base_permission.addr());
        init witnesses = Map::empty().insert(base_permission.addr(), base_permission);
        init permissions = Map::empty().insert(base_permission.addr(), base_permission);
    }
}
```

Remember, now that we have our invariants, our transitions must satisfy them after finishing. One of our invariants specified that the `base_permission` must always be present in our maps, hence, we pass it as a variable to `initialize` so we can populate the maps. Also, despite `PointsTo<StackCell>` not implementing `Copy` we are free to use it in several places with no compile-time errors. Huh? Remember, the `permissions` `storage_map` is holding the linear `tracked` permission, `addresses` and `witnesses` both create `Ghost` tokens - they are free to copy even non-copyable variables since they don't get compiled and can't be used to read a `PPtr`. Next, we need to write our most important transitions; `push` and `pop`. We'll start with push - again, hopefully nothing should be difficult to comprehend:

```rust
transition!{
    push(new_stack_cell_permission: PointsTo<StackCell>)
    {
        require(new_stack_cell_permission.is_init());
        require(pre.current_stack_addresses.last() == new_stack_cell_permission.value().next);
        require(!pre.addresses.contains(new_stack_cell_permission.addr()));
        update addresses = pre.addresses.insert(new_stack_cell_permission.addr());
        update current_stack_addresses = pre.current_stack_addresses.push(new_stack_cell_permission.addr());
        deposit permissions += [new_stack_cell_permission.addr() => new_stack_cell_permission];
        add witnesses (union)= [new_stack_cell_permission.addr() => new_stack_cell_permission];
    }
}
```

From the perspective of our TSM, pushing to the stack means depositing a permission and updating our stack representation. The permission should be initialised, have a `next` value that points to the last address in our stack representation, and should not have already been pushed to the stack. Once we have met these criteria, we update the relevant fields: add the address to the set of all addresses, push the address to the end of the stack representation, deposit the permission and add a witness. The `push` transition is relatively simple - we basically just add the permission to the TSM - now let's have a look at `pop`:

```rust
transition!{
    pop(current_head_stack_cell_permission: PointsTo<StackCell>)
    {
        require(current_head_stack_cell_permission.addr() != pre.base_address);
        require(pre.current_stack_addresses.last() == current_head_stack_cell_permission.addr());
        have witnesses >= [current_head_stack_cell_permission.addr() => current_head_stack_cell_permission];
        update popped_addresses = pre.popped_addresses.insert(pre.current_stack_addresses.last());
        update current_stack_addresses = pre.current_stack_addresses.drop_last();
    }
}
```

Wow, it's even easier! Since we are allowing `PPtr<StackCell>`s to live permanently, we never need to remove `witnesses` or `permissions`. To pop, we require that the address we are popping is not the `base_address` and that the permission is the last element in our stack representation. Once we know this, we proceed: we assert that permission actually came from the `witnesses` map, we add the address of the permission to the `popped_addresses` set and remove the last element from the stack representation.

And those are all the transitions we need. We will need a handful of properties, but those will only make sense in the context of future problems we run into. With that, it's time to take a look at the actual implementation.

### Implementation:

