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
pub struct TreiberStack {
    pub top_address: AtomicUsize,
}

pub struct StackCell {
    pub elem: u32,
    pub next_address: usize,
}
```

The following is an overview of how `push(x)` works:
 1. Atomically load the address of the top element - `top_address`.
 2. Construct a new `StackCell` with `elem = x` and `next_address = top_address`.
 3. Compare the address of the stack's current `top_address` with the previously loaded `top_address`.
 4. If the address has not changed, then no other thread has changed the stack - atomically set the stacks `top_address` to the address of your newly constructed `StackCell` and return.
 5. If the stack's `top_address` has changed, then another thread has changed the stack. Repeat from step 1.

The following is an overview of how `pop` works:
 1. Atomically load the address of the top element - `top_address`.
 2. If `top_address` is `null`, then there is nothing to pop; return.
 3. Otherwise, there is an element to pop. Cast the address to a stack cell pointer `*const StackCell` and derefence it to obtain the `elem` and `next_address`.
 4. Compare the address of the stack's current `top_address` with the previously loaded `top_address`.
 5. If the address has not changed, then no other thread has changed the stack - atomically set the stacks `top_address` to the address you previously obtained, `next_address`, and return the popped element `elem`.
 6. If the stack's `top_address` has changed, then another thread has changed the stack. Repeat from step 1.

### The ABA Problem:

Those with a keen eye might have noticed a problem with the above algorithm known as the [ABA problem](https://en.wikipedia.org/wiki/ABA_problem). The crux of the problem is as follows: we assume that the stack has not been interacted with by comparing addresses, but there is nothing stopping everything underneath this address from changing. To illustrate this point, consider the following stack:

```rust
top_address = A
*(A as *const StackCell).next_address = B
*(B as *const StackCell).next_address = C
```

Imagine that there are two threads, `T1` and `T2` interacting with the stack. The first thread `T1` beings a `pop` operation: it loads the `top_address = A` and dereferences it as a `StackCell` to get the `next_address = B`. Before it continues, it is preempted and the second thread `T2` begins execution. `T2` successfully pops the top two elements from the stack so that it then it only consists of:
```
top_address = C
```
We free the memory that is associated with `A` and `B` as they have been popped. `T2` then pushes a new element to the stack, however, because we have freed `A` & `B`'s memory, the new `StackCell` gets created at address `A`. The push proceeds and we get:
```rust
top_address = A
*(A as *const StackCell).next_address = C
```
`T2` returns and allows `T1` to continue execution. But look at what has happened! `T1` compares the address it previously loaded, `A`, with the current `top_address`, which is also A - it's a match. `T1` assumes that there has been no change and atomically set the stacks `top_address` to the address it previously obtained, `B`. This is a huge problem, `B` is no longer a valid address! The stack now looks something like this:
```rust
top_address = B
*(B as *const StackCell).next_address = ??? // Undefined behaviour as B is deallocated
```
It is clear from this that our stack is now disjoint. `C`, and anything that was in the stack below `C`, is now 'lost' - the `top_address` of the stack no longer points to a valid `StackCell`.

So how can we solve this problem?

The solution is actually somewhat trivial, but also undesireable from a performance perspective - we let `StackCell`s leak into the heap. If we never deallocate `StackCell`s, then we can never reuse their memory, and not run into this problem. Of course, as I mentioned, this is not desireable from a performance perspective - but this guide book is more focused on verification rather than performance!

## Implementation:

### Tokenized State Machine Fields:

So let's have a go at an implementation and an explanation as to why tokenized state machines are perfect for this data structure. Firstly, looking at the above pseudo-code, you'll notice that we did something like this while popping elements:
```rust
*(A as *const StackCell).elem
*(A as *const StackCell).next_address
```
You might notice that this is `unsafe`, and as a result this dereference must occur within an `unsafe` block within normal Rust - not good! Luckily, Verus has the perfect tool for bypassing this problem: [Permissioned Pointers](https://verus-lang.github.io/verus/verusdoc/vstd/simple_pptr/struct.PPtr.html).

From the source, a `PPtr<V>` is a *wrapper around a raw pointer to a heap-allocated `V`* - in our case, we will be using a `PPtr<StackCell>`. Essentially, when we create a `PPtr<V>`, we are given an `exec` pointer and a `tracked` permission. To read the underlying memory of the pointer, we must have a reference to the `tracked` permission, and to write to the underlying memory we need a mutable reference to the `tracked` permission. What is important to understand is what `tracked` means and why it is important that the permission, `PointsTo<StackCell>`, is marked with it.

When something is marked as `tracked`, that means that it will be erased at compile-time, however, Rust's linearity and borrow checking are still applied. Since `PointsTo<StackCell>` does not implement `Copy` or `Clone`, yet is marked as `tracked`, we get Rust's linearity and borrow checking guarantees on a compile-time erased type (which incurs no runtime penalty). Note, `tracked` is different to `Tracked` - wrapping a `proof` mode object in `Tracked` allows it to be passed around as an argument to functions etc. The `proof` object will still be erased at compile-time, leaving only a zero-sized `Tracked` wrapper, but it is important to understand that `tracked` is a variable mode, whereas `Tracked` is a wrapper for moving `proof` mode objects.

You'll notice that in both the `push` and `pop` algorithms, we never need to mutate any underlying data of a `StackCell`. When we create a `StackCell`, it exists as a constant until the process terminates - the only data that is mutated is the `AtomicUsize` that houses the `top_address` (which we'll discuss soon). To show you what the start of the `push` operation looks like, and how we can use permissioned pointers, we do the following (where `elem` is the element we are pushing):

```rust
// Construct a new StackCell by loading the top address
let new_stack_cell = StackCell { elem, next: self.top_address.load() };

// Wrap the new StackCell in a PPtr
// permission_guarded_new_stack_cell: PPtr<StackCell>
// new_stack_cell_permission: PointsTo<StackCell>
let (permission_guarded_new_stack_cell, Tracked(new_stack_cell_permission)) = PPtr::new(
    new_stack_cell,
);
```

So, what you need to remember is that instead of dereferencing raw pointers, we can create a permissioned pointer that points to our `StackCell`. Then, to read from the underlying data of the `PPtr<StackCell>`, we only need a reference to the related permission `PointsTo<StackCell>`. As we only need a reference, these permissions can exist in a shared data structure that all threads can access. Recall, the permissions are entirely erased at compile-time, so this shared data structure can also be compile-time erased; we can store them in a field of a tokenized state machine!

Looking through the guide, we find the [storage_map](https://verus-lang.github.io/verus/state_machines/strategy-storage-map.html) sharding strategy which even says that the tokens stored are typically of type `PointsTo<V>`. Essentially, we can shard a field in our tokenized state machine with the `storage_map` strategy, and `deposit` our permissions in this field. Then, when we want to read the data from a `PPtr<StackCell>`, we can use the `guard` command on the field to obtain a reference to the relevant `PointsTo<StackCell>`. To visualise this field, our current tokenized state machine might look something like this:

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

You'll notice that we've used a `StackCellAddress` as the key to the map. This is somewhat redundant as `PointsTo<V>` has a method `addr` that returns the address that the permission is for, but every `Map` entry requires a key, so this will do. We can tie this back in later by having an invariant asserting that every permission's address and map key are equal. When a `push` occurs, we can update the `permissions` field in our TSM to include the new permission `PointsTo<StackCell>`. Other threads can `guard` this token through the TSM to obtain a reference to the permission and read the underlying memory. 

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

It's important to understand that the `storage_map` is somewhat unique in that depositing permissions does not return a token to the user. If it did present a token to the user, then we could use that token later to `guard` the related permission; we need to implement our own 'witness' token system for retreiving references to permissions. We can use a pattern that can be seen used in various other proofs (for example [here](https://github.com/verus-lang/verus/blob/92f466f247f45128c630d1c843fd6e27d2115587/examples/state_machines/maps.rs)). In our TSM, we can add another map-like field that is identical to our storage map, with the caveat that this field holds `ghost` copies of our `tracked` tokens.

Like the `tracked` variable mode, objects that are marked with the `ghost` variable mode are erased at compile time. However, unlike the `tracked` variable mode, things that are marked as `ghost` are not checked with Rust's borrow-checker, which allows for values to be freely copied, even when they don't implement `Copy`. If we have another field that is identical to our `storage_map` value wise, but only holds `ghost` copies, then we may use those tokens as witnesses for the `tracked` tokens. This is what that looks like in practice:

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

Firstly, you'll notice that we are sharding this `witness` field with the [persistent_map](https://verus-lang.github.io/verus/state_machines/strategy-persistent-map.html) strategy. When you `add` a `ghost` key-value pair to a `persistent_map`, you are returned a token - we can use this token as our witness for the `tracked` permission. Also, the `persistent_map` strategy has no `remove` method, which is fine since we are allowing `PPtr<StackCell>`s to leak into the heap, we never need to revoke a witness. 

Secondly, you'll notice that we have an invariant asserting that the two fields are always equal. This way, we can be sure that whenever we a witness token, while we may not be able use it to directly to read from a `PPtr<StackCell>`, we can use it to `guard` a `tracked` permission and use *that* permission to read the memory. In practice, that property looks like this:


```rust
property!{
    get_permission_reference(stack_cell_address: StackCellAddress, stack_cell_permission: PointsTo<StackCell>) {
        have witnesses >= [stack_cell_address => stack_cell_permission];
        guard permissions >= [stack_cell_address => stack_cell_permission];
    }
}
```
This all hinges on the invariant that we stated in the TSM above, Verus knows that the maps are equal and therefore it knows that if we have a `(StackCellAddress, PointsTo<StackCell>)` pair in our `witnesses` map, then there must be an equal pair in the `permissions` map. The only difference is that the `permissions` map 'stores' the `tracked` value that is useable.

So far, you might notice that all of our fields are sharded with map strategies. Since we require both of these maps to be equal, we know that when we want to insert a key-value pair, we must demonstrate the more binding requirement out of the two. i.e. to insert a key `k` into these maps, we must show:
```rust
!permissions.dom().contains(k);
```

But there is one problem with this, since the `permissions` field is sharded with the `storage_map` strategy, attempting to naively `require` this fact in a transition results in an error. The following is an example transition that reflects how we might try to show that a key is not in the map:

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

How will another field solve our problems? Well, we can have a field with type `set` and shard it with the [variable](https://verus-lang.github.io/verus/state_machines/strategy-variable.html) strategy with an invariant that this set is equal to the domain of our maps. Now, when we want to insert into the maps, we only need to `require` that the key is not within the set - a legal expression! Here is an example of what that field, invariant and transition might look like:

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

The point of this, is that the this `variable` sharded `set` serves as a birds-eye view of all the tokens from the `persistent_map`. Even if we held all of the tokens from the TSM in one location, and presented them to the TSM when we called the transition, we still wouldn't have a way to assert that this collection of tokens truly contains every single token ever minted; the `variable` `set` sidesteps this problem. The downside of using this method is that there is one singular `addresses` token, so we may not give multiple threads mutable access to it. This sounds like a problem, but as you'll see later, we don't need to give mutable references to this token to multiple threads.

On top of this, it is also required in our TSM to have a field that reflects our real `exec` stack. If this field represents our stack, and we only ever interact with this field through stack operations (push and pop) within transitions, then we can be sure that our implementation is functionally correct.

> **Note:**
>
> This is not entirely true, but for the simplicity of the guide it is sufficient. For a more rigorous proof, the user would also have to include a linearised history of events and return a witness token to the user after every operation. The purpose of the witness token is to give proof to the user that their intended action actually happened. It might be confusing, but this witness token is different to the kind we have already been using...
>
> Essentially, without a linearised history, the program could technically just do nothing when a `push` or a `pop` is called, and this would satisfy all specifications provided (provided that the intial state satisfies them). Adding these new witness tokens demonstrates that something has actually occurred, but adding them is too complicated for this guide.
>
> Unless you want to be very rigorous, you can forget that I said anything!

So, to add those stack-representation fields to our TSM, we might do something like this:

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

Great, we already have a fair few invariants - let's have a think about what else we need to include. Recall that we are using a `storage_map` to shard `permissions`, and a `persistent_map` to shard `witnesses`. It was mentioned earlier that the key of each key-value pair can be the associated address of the value. That is to say:

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

Remember, now that we have our invariants, our transitions must satisfy them after finishing. One of our invariants specified that the `base_permission` must always be present in our maps, hence, we pass it as a variable to `initialize` so we can populate the maps. Also, despite `PointsTo<StackCell>` not implementing `Copy` we are free to use it in several places with no compile-time errors. Huh? Remember, the `permissions` `storage_map` holds the linear `tracked` permission, while `addresses` and `witnesses` both create tokens whose value is `ghost` and hence freely copyable. Next, we need to write our most important transitions; `push` and `pop`. We'll start with push - again, hopefully nothing should be difficult to comprehend:

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

From the perspective of our TSM, pushing to the stack means depositing a permission and updating our stack representation. The permission should be initialised, have a `next` value that points to the last address in our stack representation `Seq`, and should not have already been pushed to the stack. Once we have met these criteria, we update the relevant fields: add the address to the set of all addresses, push the address to the end of the stack representation, deposit the permission and add a witness. The `push` transition is relatively simple - we basically just add the permission to the TSM - now let's have a look at `pop`:

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

Wow, it's even easier! Since we are allowing `PPtr<StackCell>`s to live permanently, we never need to remove `witnesses` or `permissions`. To pop, we require that the address we are popping is not the `base_address` and that the permission is the last element in our stack representation. Once we know this, we proceed: we assert that the permission is a member of the `witnesses` map, we add the address of the permission to the `popped_addresses` set and remove the last element from the stack representation.

And those are all the transitions we need. We will need a handful of properties, but those will only make sense in the context of future problems we run into.

#### Tokenized State Machine Purpose:

So what does the tokenised state machine actually buy us? Well, there are two key parts of the state machine that we have created; the `StackCell` permission storage, and the stack representation.

First, regarding the permissions, the TSM holds the permissions for all `StackCell`s that have ever been pushed to the stack. It is important to note, both the TSM - and therefore the permissions that it holds - are erased at compile-time. Hence, the TSM doesn't physically store anything, but it mimics something that does. At the call site of `push` we are given a witness token (also compile-time erased) that the permission is now included in the TSM. This token is sufficient proof at the call site of `pop` that a permission exists for `top_address`. Hence, we may safely dereference (read from a `PPtr`) `top_address` into a `StackCell`. All of these checks are done at compile time and are facilitated by the TSM.

Second, regarding the stack representation `current_stack_addresses`, the TSM holds a `Seq<StackCellAddress>` that represents the physical stack. Note, the TSM alone does not relate it's representation to the physical stack, that is something that we will have to do ourselves. However, if we can relate this representation to the `exec` stack, and this representation is only interacted with via the `push` and `pop` transitions, then our stack is functionally correct.

With all of this in mind, lets take a look at how we can relate these invariants and state to the implementation of a stack.

### Well-Formedness:

For the implementation, we already have the foundation of what we will use:

```rust
use std::sync::atomic::AtomicUsize;

pub struct TreiberStack {
    pub top_address: AtomicUsize,
}

pub struct StackCell {
    pub elem: u32,
    pub next_address: usize,
}
```

But I just mentioned that there is no relation from this implementation and our state machine. So how can we tie these together? Well, we can make use of Verus' [AtomicUsize](https://verus-lang.github.io/verus/verusdoc/vstd/atomic_ghost/struct.AtomicUsize.html) and [struct_with_invariants](https://verus-lang.github.io/verus/verusdoc/vstd/pervasive/macro.struct_with_invariants.html) macro! So how do these work? We can start by having a look at the `AtomicUsize` type definition and `new` method:

```rust
impl<K, G, Pred> AtomicUsize<K, G, Pred> {

    pub const exec fn new(
        Ghost(k): Ghost<K>,
        u: usize,
        Tracked(g): Tracked<G>,
    ) -> t : Self
        requires Pred::atomic_inv(k, u, g),
        ensures t.well_formed() && t.constant() == k
}
```

So what is going on here? First, there are three type parameters `K`, `G` and `Pred` and interestingly none of these are the actual `usize` that we store in the atomic. Taking a look at the `new` method, we can fill in some blanks: `K` is some kind of `Ghost` state & `G` is some kind of `Tracked` state (both of these will be erased at compile time), which leaves `u` as the physical `usize`. Then, in the `requires` clause, we can see that `Pred` implements `atomic_inv` which holds an invariant on this state as well as the actual `u: usize`. It looks like, we pass some state (both `ghost` and `tracked`), along with an `exec` `usize` and an invariant that ties this all together.

Jumping to the `struct_with_invariants!` macro, it states: *The struct_with_invariants! macro is used at the item level, and it should contains a single struct declaration followed by a single declaration of a spec function returning bool. However, this spec function should not contain a boolean predicate as usual, but instead a series of invariant declarations.* Then later: *A field of the struct, if it uses a supported type, may leave the type incomplete by omitting some of its type parameters*. 

Essentially, we may define our `AtomicUsize` to have type parameters `AtomicUsize<_, G, _>` where `G` is the type of the `tracked` token/collection of tokens from the above definition. Then, we define our predicate inside the `struct_with_invariants!` (for our case we will call it `wf` and refer to it as 'well-formed') and this will serve as the invariant that relates the physical `usize` our atomic holds and the `tracked` tokens. But wait, what about the last argument, what does the `Ghost<K>` do? This serves as a ghost constant that we may also use in our well-formedness invariant. 

It is useful in proofs to have state that is constant to serve as 'anchors'. For example, we have one `base_address` that we create when we initialise the stack, without some constant state we would need to assert that this variable is unchanged quite often. Also, it is useful to assert that all of the tokens that we hold are not only all from the same TSM instance, but also from the same TSM that we have been using the whole time. The following example shows how to use this constant state:

```rust
struct_with_invariants!{
    pub struct NumberHolder {
        pub number: u32,
        pub atomic_size: AtomicUsize<_, (), _>,
    }

    pub open spec fn wf(self) -> bool {
        invariant on atomic_size with (number) is (size: usize, nothing: ()) {
            true
        }
    }
}
```

In the above example, we've define a struct `NumberHolder` and a related well-formedness condition inside the `struct_with_invariants!` macro. You'll notice in `wf` that the invariant is on the `atomic_size` field of the struct, but we also want to use the `number` field as well, so we include it in the `with` clause. Note, if we include other fields in the struct but do not include them in the `with` clause, then they will not be constant. The `is` clause contains the unpacked exec `usize` and also the `Tracked` state (for this example, we aren't tracking anything, so we can use unit). You'll also notice that the invariant simply returns `true` and so it should always pass. Let's give it a go:

```rust
pub fn main() {
    let ghost_k: Ghost<u32> = Ghost(5);
    let size: usize = 0;
    let tracked_g: Tracked<()> = Tracked(());

    let atomic_size = AtomicUsize::new(
        ghost_k,
        size,
        tracked_g,
    );

    let number_holder = NumberHolder { number: 6, atomic_size };
    assert(number_holder.wf());
}
```

Everything looks good, this should pass right? Let's try verifying it:

```rust
error: assertion failed
  --> ..\small_invariant_test.rs:XX:XX
   |
XX |     assert(number_holder.wf());
   |            ^^^^^^^^^^^^^^^^^^ assertion failed
```

That's strange, our well-formedness condition should always hold, so why is it failing? Notice that we defined the `AtomicUsize` to have a ghost constant `K: u32` with value `5` at construction. Then, when we later instantiated `number_holder`, we defined `number_holder.number = 6`. You might think this to be fine because we made no assertions about the `number` field in our invariant `NumberHolder::wf` - but we didn't have to, this is one of Verus' hidden proof obligations.

When we construct the `AtomicUsize` with this constant ghost state `Ghost(5)`, the `struct_with_invariants!` requires a field in the struct that 'mirrors' this constant. That is why, in the above example, we had to include the `number` field in the well-formedness condition regardless of the fact that we did not use it. i.e. the following does not compile:

```rust
struct_with_invariants!{
    pub struct NumberHolder {
        pub number: u32,
        pub atomic_size: AtomicUsize<_, (), _>,
    }

    pub open spec fn wf(self) -> bool {
        invariant on atomic_size with () is (size: usize, nothing: ()) {
            true
        }
    }
}

pub fn main() {
    let ghost_k: Ghost<u32> = Ghost(5);
    let size: usize = 0;
    let tracked_g: Tracked<()> = Tracked(());

    let atomic_size = AtomicUsize::new(
        ghost_k,
        size,
        tracked_g,
    );

    let number_holder = NumberHolder { number: 6, atomic_size };
}
```

Attemping to compile gives the following error:

```rust
  --> ..\small_invariant_test.rs:XX:XX
   |
XX |     let number_holder = NumberHolder { number: 6, atomic_size };
   |                                                   ^^^^^^^^^^^ expected `AtomicUsize<(), (), ...>`, found `AtomicUsize<u32, (), _>`
   |
```

I.e. Verus determines the type parameters of the `AtomicUsize` based on what is included in the the invariant's `with` and `is` clauses:
```rust
invariant on atomic_size with (number) is (size: usize, nothing: ())
```
Ok, we now know about the types of the fields, but that still doesn't explain why the `number_holder.wf()` is failing. Well, as I mentioned above, the `struct_with_invariants!` requires a field in the struct that 'mirrors' each constant included when the atomic is created. Then, Verus checks that each 'constant-mirroring' field in the struct equals the constant that was passed in when the Atomic was created without us needing to check this in `wf`. In fact, we don't even have the option; Verus does not expose a method to read these constant values inside of the invariant, Verus just checks that they are equal and then you are free to use your struct-defined-fields instead. Essentially, you create your atomic with some constant ghost state and then whenever you check if your struct is well-formed, Verus checks if your constant fields equal the state defined at initialisation. Be careful, you are free to mutate your fields however you like, as long as they return to the constant values before you call a `wf` again. With this in mind, we can fix our above example, like so:

```rust
pub fn main() {
    let ghost_k: Ghost<u32> = Ghost(5);
    let size: usize = 0;
    let tracked_g: Tracked<()> = Tracked(());

    let atomic_size = AtomicUsize::new(
        ghost_k,
        size,
        tracked_g,
    );

    let number_holder = NumberHolder { number: 5, atomic_size };
    assert(number_holder.wf()) // Passes
}
```

With this in mind, we will use the constant `ghost` state to keep track of things like the `base_address` and TSM `instance`. However it is the mutable `tracked` state that we will use to relate the TSM tokens to the `exec` `usize` in the `AtomicUsize`. 

The idea is as follows: the TSM's tokens + invariants are useful for maintaining system-wide properties. There is no requirement to use a TSM when you are using permissioned-pointers, but by virtue of logging all permissions with the TSM, we can assert system-wide invariants. The physical `TreiberStack` object only contains information about the `top_address`. We store the tokens from our TSM in the `tracked` state associated with the `AtomicUsize` and use the well-formedness condition to restate not only the system-wide invariants, but relate them to the `top_address`. The combination of the correct invariants and well-formedness condition is enough to assert that the `TreiberStack` holds a valid `top_address` and every time this address updates, it upholds the stack-like properties of `push` and `pop`.

So, let's think about what we want to include in our `AtomicUsize`. Starting with the constant ghost state, I mentioned above that we want to keep track of the TSM instance to be sure that all of our tokens are from the correct TSM, and we also want to keep track of the base address.

Moving on to the `Tracked` but mutable state, what we want to keep here should hopefully be clear to the reader: the tokens from our TSM. Remember, this `AtomicUsize` facilitates the relation between the TSM and our physical address, hence, we may store a struct that holds multiple tokens from our TSM. Given this, we may update our previous struct and write a skeleton well-formedness condition: 

```rust
pub tracked struct AtomicTokens {
    pub current_stack_addresses: machine::current_stack_addresses,
    pub popped_addresses: machine::popped_addresses,
    pub witnesses: Map<StackCellAddress, machine::witnesses>,
    pub addresses: machine::addresses,
}

struct_with_invariants!{
    pub struct TreiberStack {
        pub base_address: StackCellAddress,
        pub top_address: AtomicUsize<_, AtomicTokens, _>,
        pub instance: Tracked<machine::Instance>,
    }

    pub open spec fn wf(self) -> bool {
        invariant on top_address with (base_address, instance) is (top_addr: usize, atomic_tokens: AtomicTokens) {
            true
        }
    }
}
```

First, you'll notice that our aptly named `AtomicTokens` struct has four fields - one for each field in our TSM that creates a token. As explained above, this will be the connection from our TSM to our physical code, so we need an entry for each field of the TSM that creates tokens. Also, we've marked the struct as `tracked` - this property propagates to all the fields so that we don't have to wrap everything in `Tracked`.

> **Note:**
>
> You'll also notice that we are storing a `Map<StackCellAddress, machine::witnesses>`, whereas all the other tokens are stored without extra structure. If you recall, the `witnesses` field is sharded as a `persistent_map` and, as a result, tokens exist for every key-value pair rather than the one for the field as a whole. Hence, since we are storing a collection of these tokens, we need a set-like structure that can store our collection; a `Map` works best. One key-value pair in this map may look like: `(address, (address, permission))`. Is this clunky? Yes. Do I hate it? Also, yes. But does it work. Yes.

Secondly, you'll notice that our well-formedness condition is always true and would only fail if the `base_address` or `instance` has changed. So what should we include - what does it mean for our stack to be well-formed? Interestingly, we don't actually have a holistic view of the stack even here, we only have a view of the `top_address`. Of course, we have our stack representation token `current_stack_addresses`, but we do not have a full view of the entire physical stack. We can start with a few basic checks, for example the `base_address` stored in our struct should equal the `base_address` from our TSM. Further, all tokens stored in our struct should actually be from the correct TSM instance:

```rust
// The base address must reflect the TSM base address:
&&& base_address == instance.base_address()

// All tokens must come from the correct TSM:
&&& atomic_tokens.current_stack_addresses.instance_id() == instance.id()
&&& atomic_tokens.popped_addresses.instance_id() == instance.id()
&&& atomic_tokens.addresses.instance_id() == instance.id()
&&& forall |addr: StackCellAddress| #![auto]
        atomic_tokens.witnesses.dom().contains(addr) ==>
            atomic_tokens.witnesses.index(addr).instance_id() == instance.id()
```
You might think that the `base_address == instance.base_address()` check is unnecessary - after all, Verus will check if `base_address` differs from the value we use to initialise the `AtomicUsize`, surely we don't need this check as long as it is true at initialisation. This is correct, however Verus only discharges two separate the assertions that `base_address` and `instance` have not changed. Any relation between the pair of values must be instantiated again for use in the proof. 

We can also assert that our `Map` structure storing the`witness tokens is setup correctly. i.e. the key of each key-value pair is equal to the key of the key-value pair in the value (yes, that is confusing, but its not too bad to read):

```rust
// Each individual map entry must agree internally at the address it is referencing (map structure):
forall |addr: StackCellAddress| #![auto]
    atomic_tokens.witnesses.dom().contains(addr) ==> (
        atomic_tokens.witnesses.index(addr).key() == addr &&
        atomic_tokens.witnesses.index(addr).value().addr() == addr
    )
```

With those structural/basic properties out of the way, there are two more categories of invariant clauses we would like to assert: invariants regarding correctness, and invariants that allow us to use the TSM. Starting with invariants regarding correctness, we want to be sure that the stack representation token `current_stack_addresses` has the `base_address` at the bottom, and the `top_addr` at the top:

```rust
// The base address is always present even before the first push:
&&& atomic_tokens.witnesses.dom().contains(base_address)
&&& atomic_tokens.current_stack_addresses.value().contains(base_address)
&&& atomic_tokens.current_stack_addresses.value().first() == base_address

// The top address is always tracked:
&&& atomic_tokens.witnesses.dom().contains(top_addr)
&&& atomic_tokens.current_stack_addresses.value().contains(top_addr)
&&& atomic_tokens.current_stack_addresses.value().last() == top_addr
```

We also know, that if there is only one element in our stack representation, then our stack is empty (because the `base_address` is always included in the stack representation). Hence:

```rust
// If the top is the base, then our stack is empty (we only have the base):
&&& top_addr == base_address <==> atomic_tokens.current_stack_addresses.value().len() == 1
```

And that's most of what we need for the stack to be considered correct - remember, the only physical thing that we can reference is the address of the top `StackCell`, `top_addr`. Despite this, we also need to assert a few clauses that our TSM asserts. The reasoning for this is simple, when the TSM mints new tokens, the invariants hold. However, Verus does not discharge these invariants everywhere and hence we 'lose' facts about our tokens unless we explicitly state them as invariant. Also, Verus also requires that the TSM's invariants hold on any tokens that we present when using a transition; if we have lost these facts then our transitions can't be used. Hence, we may include the following invariants in our well-formedness condition that come from the TSM. I won't explain in great detail what they do, since we've talked about them already in the TSM section:

```rust
// There are no duplicate addresses in our stack
&&& atomic_tokens.current_stack_addresses.value().no_duplicates()

// The current stack cell addresses is disjoint from the set of all popped stack cell addresses:
// However, their union should be the domain of the set of all witnesses
&&& atomic_tokens.current_stack_addresses.value().to_set().disjoint(atomic_tokens.popped_addresses.value())
&&& atomic_tokens.witnesses.dom() =~= atomic_tokens.current_stack_addresses.value().to_set().union(atomic_tokens.popped_addresses.value())
&&& atomic_tokens.current_stack_addresses.value().to_set().subset_of(atomic_tokens.witnesses.dom())

// The set of cell addresses should equal the domain of the witness tokens:
&&& atomic_tokens.addresses.value() == atomic_tokens.witnesses.dom()

// Every witness token's permission points to initialised memory except for the witness of the base address:
&&& forall |addr: StackCellAddress| #![auto]
        atomic_tokens.witnesses.dom().contains(addr) ==> (
            addr != base_address <==> atomic_tokens.witnesses.index(addr).value().is_init()
        )

// There exists a witness for the next stack cell of every current stack cell (except base):
&&& forall |addr: StackCellAddress| #![auto]
        (
            atomic_tokens.witnesses.dom().contains(addr) &&
            addr != base_address
        ) ==>
        atomic_tokens.witnesses.dom().contains(
            atomic_tokens.witnesses.index(addr).value().value().next_address
        )

&&& forall |i: int| #![auto]
        0 < i < atomic_tokens.current_stack_addresses.value().len() ==> (
            atomic_tokens.current_stack_addresses.value()[i-1] ==
            atomic_tokens.witnesses.index(atomic_tokens.current_stack_addresses.value()[i]).value().value().next_address
        )
```

That is everything that we need for our stack to be well-formed. We have system-wide invariants from the TSM through the tokens, and a relation between these tokens and `top_address` through the `AtomicUsize`s stored state and well-formedness condition (provided that we assert this condition). Let's move on to the real stuff!

### Construction:

We haven't actually written a `push` or `pop` method yet, but we've done most of the heavy lifting in the proof already. Let's start by writing the method that constructs a new `TreiberStack`: we know that this method should not take any arguments and should return to us a fresh `TreiberStack` that is well formed:

```rust
pub fn new() -> (treiber_stack: Self)
    ensures
        treiber_stack.wf()
{
    // TODO
}
```

We can start by initialising the TSM. But before we do so, recall that `initialize` requires a permission for the `base_address`. On top of this, the initialisation of a `storage_map` also requires an input of type `Map<K, Tok>`. In our case, since `initialize` inserts the `base_address` and permission in to the map, we must pass in a map that has this inserted. Hence, our `new` method now starts like so:

```rust
pub fn new() -> (treiber_stack: Self)
    ensures
        treiber_stack.wf()
{
    let (base, Tracked(base_perm)) = PPtr::<StackCell>::empty();
    let base_address = base.addr();

    let tracked permissions = Map::tracked_empty();
    proof {
        permissions.tracked_insert(base_address, base_perm);
    }

    let tracked (
        Tracked(instance),
        Tracked(current_stack_addresses),
        Tracked(popped_addresses),
        Tracked(addresses),
        Tracked(witnesses),
    ) = machine::Instance::initialize(base_perm, permissions);

    // TODO
}
```

Now, all that's left to do is construct our struct of `AtomicTokens`, initialise the `AtomicUsize` and return a `TreiberStack` constructed from these. Combining these, we get:

```rust
pub fn new() -> (treiber_stack: Self)
    ensures
        treiber_stack.wf(),
{
    let (base, Tracked(base_perm)) = PPtr::<StackCell>::empty();
    let base_address = base.addr();

    let tracked permissions = Map::tracked_empty();
    proof {
        permissions.tracked_insert(base_address, base_perm);
    }

    let tracked (
        Tracked(instance),
        Tracked(current_stack_addresses),
        Tracked(popped_addresses),
        Tracked(addresses),
        Tracked(witnesses),
    ) = machine::Instance::initialize(base_perm, permissions);

    let tracked witnesses = witnesses.into_map();

    let tracked atomic_tokens = AtomicTokens {
        current_stack_addresses,
        popped_addresses,
        witnesses,
        addresses
    };

    assert(current_stack_addresses.value().first() == base_address);

    let top_address = AtomicUsize::new(
        Ghost((base_address, Tracked(instance))),
        base_address,
        Tracked(atomic_tokens),
    );

    TreiberStack { base_address, top_address, instance: Tracked(instance) }
}
```

Remember, we need the `base_address` and TSM `instance` as constants, so we pass them as a `(base_address, Tracked(instance))` to the `AtomicUsize`. We also initialise it with `base_address` as the starting physical `usize` and the `atomic_tokens` struct that will hold tokens from our TSM.

And with that, we have arrived at where most of you likely thought we would start; writing `push` and `pop` operations.

### Push:

Lets start with the basics. We know that we are pushing an element of some kind to the stack - we will be using a `u32`. We also know that `push` will loop forever, but Verus won't allow us to do this normally, so we'll have to include `#![cfg_attr(verus_keep_ghost, verifier::exec_allows_no_decreases_clause)]` at the top of our file. Trivially, we will have to construct a new `StackCell` and wrap it in a `PPtr` as well. Finally, we want the stack to be well-formed both when we start and when we finished. Combining, we can start with:

```rust
pub fn push(&self, elem: u32)
    requires
        self.wf(),
    ensures
        self.wf(),
{
    loop
        invariant
            self.wf(),
    {
        let new_stack_cell = StackCell { elem, next_address: self.top_address.load() };
        let (permission_guarded_new_stack_cell, Tracked(new_stack_cell_permission)) = PPtr::new(
            new_stack_cell,
        );

        // TODO
    }
}
```

Now that we have loaded the `top_address` and constructed a new permission-wrapped `StackCell`, we can finally attempt to push. Naively, that might look something like this:

```rust
self.top_address.compare_exchange(
    permission_guarded_new_stack_cell.read(Tracked(&new_stack_cell_permission)).next_address,
    permission_guarded_new_stack_cell.addr()
);
```

We compare what is stored in `self.top_address` with what we previously loaded, if the value is still the same, then we exchange the address of our new cell `permission_guarded_new_stack_cell.addr()` with the `top_address`. However, there is a problem with this - our well-formedness condition will not pass as we haven't used our TSM, nor have we updated any of our tokens in the `AtomicUsize`. We want to synchronise this atomic action with a TSM-transition and ghost token update. So what does Verus have to allow us to synchronise this atomic action with a ghost action? The [atomic_with_ghost](https://verus-lang.github.io/verus/verusdoc/vstd/atomic_ghost/macro.atomic_with_ghost.html) macro!

Essentially, this macro allows us to synchronise an `exec` atomic action with a series of ghost actions. Since the actions are all ghost, we may perform multiple of them and still have the overall effect be synchronised with a single atomic action. I.e. we can call multiple TSM-transitions, TSM-properties and ghost field updates 'at the same time' as an atomic action with the goal being that our updated `AtomicUsize` still passes the well-formedness check. Lets first take a look at how the `atomic_with_ghost` macro looks:

```rust
atomic_with_ghost!(
    self.top_address => compare_exchange(
        permission_guarded_new_stack_cell.read(Tracked(&new_stack_cell_permission)).next_address,
        permission_guarded_new_stack_cell.addr()
    );
    returning previous_head_address_result;
    ghost atomic_tokens => {
        if let Ok(_) = previous_head_address_result {
            // Token Update
        }
    }
);
```

There are three things that are happening here. First, give the macro the `exec` atomic action:
```rust
self.top_address => compare_exchange(
    permission_guarded_new_stack_cell.read(Tracked(&new_stack_cell_permission)).next_address,
    permission_guarded_new_stack_cell.addr()
);
```
Second, we make the returned value of this atomic action accessible within the scope of the ghost update:
```rust
returning previous_head_address_result;
```
Taking a look at the specification for `compare_exchange(x, y)`, we see that `previous_head_address_result` will take the following form:
```rust
prev == x && next == y && ret == Ok(prev) //success
OR 
prev != x && next == prev && ret == Err(prev) //failure
```
Finally, we access the ghost-state associated with our `AtomicUsize` by unpacking it and naming it `atomic_tokens`. Also, since we know that we will only update our state if the CAS was successful, we only need to account for this case:
```rust
ghost atomic_tokens => {
    if let Ok(_) = previous_head_address_result {
        // Token Update
    }
}
```
Great, so if we have reached this point, then our CAS was successful - so how do we need to update the state such that our `AtomicUsize` is well-formed? Remember that part of the purpose of the TSM is to mirror the actions that are taken by our `exec` stack, and we have already constructed a `push` transition in our TSM which returns a new token back to the user. In an ideal world, we would only need to call this transition and then add the token we received to the `AtomicUsize`'s associated state:
```rust
ghost atomic_tokens => {
    if let Ok(_) = previous_head_address_result {
        let tracked witness_token = self.instance.push(
            new_stack_cell_permission,
            &mut atomic_tokens.current_stack_addresses,
            &mut atomic_tokens.addresses,
            new_stack_cell_permission
        );

        atomic_tokens.witnesses.tracked_insert(witness_token.key(), witness_token);
    }
}
```

However, this alone will not compile. Verus complains as it cannot prove that the `new_stack_cell_permission` we are depositing into the state machine is not already present. Of course, we know that it is not already present since we just constructed this `PPtr` and `PointsTo<StackCell>` moments ago - so how can we convince Verus that this is unique? We may add a [property](https://verus-lang.github.io/verus/state_machines/properties.html) in the TSM and use it to derive a contradication!

Our argument goes like this:
 1. We own a `tracked` `PointsTo` which is linearly typed.
 2. We deposit `tracked` `PointsTo`s into our TSM, and we may later obtain a reference to any such `PointsTo`.
 3. Verus cannot (without help) guarantee that the `PointsTo` that we hold isn't already in the TSM.
 4. If the `PointsTo` that we hold is already in the TSM, then we may obtain a reference to it.
 5. Now we have two distinct (since they are linear) `PointsTo` permissions to the same memory region - contradiction.
 6. Therefore, the `PointsTo` that we hold is not already in the TSM.

To actually use this argument, we add the following `property` to our TSM:

```rust
property!{
    same_address_implies_same_permission(stack_cell_permission_1: PointsTo<StackCell>, stack_cell_permission_2: PointsTo<StackCell>) {
        require(stack_cell_permission_1.addr() == stack_cell_permission_2.addr());
        have witnesses >= [stack_cell_permission_1.addr() => stack_cell_permission_1];
        have witnesses >= [stack_cell_permission_2.addr() => stack_cell_permission_2];
        assert(stack_cell_permission_1 == stack_cell_permission_2);
    }
}
```

This property doesn't alone provide the contradiction, but it does export the fact that if we have two permissions that point to the same region of memory, then the permissions are equal. To use this fact in our `atomic_with_ghost` macro, we add the following logic:

```rust
if atomic_tokens.witnesses@.dom().contains(new_stack_cell_permission.addr()) {
    let tracked witness_token = atomic_tokens.witnesses.tracked_borrow(new_stack_cell_permission.addr());
    let tracked stack_cell_permission_reference = self.instance.get_permission_reference(witness_token.key(), witness_token.value(), &witness_token);
    new_stack_cell_permission.is_distinct(stack_cell_permission_reference);
    assert(false);
}
```

If our `AtomicUsize`'s state `atomic_tokens` contains a witness token with address equal to our newly constructed permission's address, then we present this token and our new token to the property we just defined. This property exports that the two permissions that we have are equal, however we make use of the [is_distinct](https://verus-lang.github.io/verus/verusdoc/vstd/simple_pptr/struct.PointsTo.html#method.is_distinct) proof to show that these permissions must be distinct. Combining these, we may `assert(false)` which informs Verus that our initial assumption was incorrect - i.e. Our `AtomicUsize`'s state `atomic_tokens` does not contain a witness token with address equal to our newly constructed permission's address. From here, we are free to use our TSM's transition as before.

From here, all we have to do is return - our `push` operation is complete as we have synced our `exec` CAS with correctly updating our associated ghost state through the TSM.

> **Note:**
> There are a few other minor assertions that we need to discharge to Verus, but these are all trivial and not related to the generaly proof structure.
>

Our final `push` operation looks like this:

```rust
pub fn push(&self, elem: u32)
    requires
        self.wf(),
    ensures
        self.wf(),
{
    loop
        invariant
            self.wf(),
    {
        let new_stack_cell = StackCell { elem, next_address: self.top_address.load() };
        let (permission_guarded_new_stack_cell, Tracked(new_stack_cell_permission)) = PPtr::new(
            new_stack_cell,
        );

        let mut push_result =
            atomic_with_ghost!(
            self.top_address => compare_exchange(
                permission_guarded_new_stack_cell.read(Tracked(&new_stack_cell_permission)).next_address,
                permission_guarded_new_stack_cell.addr()
            );
            returning previous_head_address_result;

            ghost atomic_tokens => {
                if let Ok(_) = previous_head_address_result {

                    // Proving that there does not already exist a permission for the cell in the TSM (or our tokens by extension):
                    if atomic_tokens.witnesses@.dom().contains(new_stack_cell_permission.addr()) {
                        let tracked witness_token = atomic_tokens.witnesses.tracked_borrow(new_stack_cell_permission.addr());
                        let tracked stack_cell_permission_reference = self.instance.get_permission_reference(witness_token.key(), witness_token.value(), &witness_token);
                        new_stack_cell_permission.is_distinct(stack_cell_permission_reference);
                        assert(false);
                    }

                    let ghost pre_current_stack_addresses = Ghost(atomic_tokens.current_stack_addresses@.value());

                    let tracked witness_token = self.instance.push(
                        new_stack_cell_permission,
                        &mut atomic_tokens.current_stack_addresses,
                        &mut atomic_tokens.addresses,
                        new_stack_cell_permission
                    );

                    assert(pre_current_stack_addresses@ =~= pre_current_stack_addresses.push(witness_token.value().addr()).drop_last());

                    // Insert the witness token for the new stack cell into our map:
                    atomic_tokens.witnesses.tracked_insert(witness_token.key(), witness_token);

                    // The push correctly updated our view of the stack:
                    assert(atomic_tokens.current_stack_addresses.value().last() == witness_token.key());
                }
            }
        );

        if let Ok(_) = push_result {
            return;
        }
    }
}
```

### Pop:
Our `pop` method has the same skeleton as `push`; we loop infinitely until we successfully pop an element:
```rust
pub fn pop(&self) -> (elem: Option<u32>)
    requires
        self.wf(),
    ensures
        self.wf(),
{
    loop
        invariant
            self.wf(),
    {
       // TODO
    }
}
```
However, you may recall that we need to do two checks when we pop. First, we check if the stack is empty and return nothing to the user if this is the case. Second, if the stack is not empty, then we derefence the `StackCell` address to get the `next` address which we will replace the old `top_address` if the CAS is successful. Lets first look at the empty-stack check:

```rust
let tracked stack_head_witness;

let mut top_address =
    atomic_with_ghost!{
    self.top_address => load();
    returning addr;

    ghost atomic_tokens => {
        stack_head_witness = *atomic_tokens.witnesses.tracked_borrow(addr);
    }
};

if top_address == self.base_address {
    return None;
}
```