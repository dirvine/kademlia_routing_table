// Copyright 2015 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net Commercial License,
// version 1.0 or later, or (2) The General Public License (GPL), version 3, depending on which
// licence you accepted on initial access to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project generally, you agree to be
// bound by the terms of the MaidSafe Contributor Agreement, version 1.0.  This, along with the
// Licenses can be found in the root directory of this project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.
//
// Please review the Licences for the specific language governing permissions and limitations
// relating to use of the SAFE Network Software.

//! # Kademlia Routing Table
//!
//! Kademlia Routing Table implementation.

#![doc(html_logo_url =
           "https://raw.githubusercontent.com/maidsafe/QA/master/Images/maidsafe_logo.png",
       html_favicon_url = "http://maidsafe.net/img/favicon.ico",
       html_root_url = "http://maidsafe.github.io/kademlia_routing_table")]

// For explanation of lint checks, run `rustc -W help` or see
// https://github.com/maidsafe/QA/blob/master/Documentation/Rust%20Lint%20Checks.md
#![forbid(bad_style, exceeding_bitshifts, mutable_transmutes, no_mangle_const_items,
          unknown_crate_types, warnings)]
#![deny(deprecated, drop_with_repr_extern, improper_ctypes, missing_docs,
        non_shorthand_field_patterns, overflowing_literals, plugin_as_library,
        private_no_mangle_fns, private_no_mangle_statics, stable_features, unconditional_recursion,
        unknown_lints, unsafe_code, unused, unused_allocation, unused_attributes,
        unused_comparisons, unused_features, unused_parens, while_true)]
#![warn(trivial_casts, trivial_numeric_casts, unused_extern_crates, unused_import_braces,
        unused_qualifications, unused_results, variant_size_differences)]
#![allow(box_pointers, fat_ptr_transmutes, missing_copy_implementations,
         missing_debug_implementations)]

#[macro_use]
extern crate log;

#[macro_use]
#[allow(unused_extern_crates)]
extern crate maidsafe_utilities;

extern crate itertools;
#[cfg(test)]
extern crate rand;
extern crate xor_name;

use itertools::*;

/// Defines the size of close group
pub const GROUP_SIZE: usize = 8;

/// Quorum size.
pub const QUORUM_SIZE: usize = 5;

/// Defines the number of contacts which should be returned by the `target_nodes` function for a
/// target which is outwith our close group and is not a contact in the table.
pub const PARALLELISM: usize = 4;

/// Defines the target max number of contacts per bucket.  This is not a hard limit; buckets can
/// exceed this size if required.
const BUCKET_SIZE: usize = 1;

/// Defines the target max number of contacts for the whole routing table.  This is not a hard limit;
/// the table size can exceed this size if required.
const OPTIMAL_TABLE_SIZE: usize = 64;

/// required trait for the info held on a node by routing_table
pub trait HasName {
    /// return xor_name for this type
	fn name(&self) -> &::xor_name::XorName;
}

/// Allows user defined data for routing table
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeInfo<T, U> {
	/// container for node in routing table
    pub public_id: T,
	/// connection object, may be socket etc.
    pub connections: Vec<U>,
}

impl <T: PartialEq + HasName + ::std::fmt::Debug, U : PartialEq> NodeInfo<T, U> {
   /// constructor
	pub fn new(public_id: T, connections: Vec<U>) -> NodeInfo<T, U> {
        NodeInfo {
            public_id: public_id,
            connections: connections,
        }
    }

    /// name of routing table entry
    pub fn name(&self) -> &::xor_name::XorName {
        self.public_id.name()
    }
}

// Internal struct for routing
#[derive(Clone, Debug, PartialEq, Eq)]
struct RoutingNode<T, U> {
    node_info : NodeInfo<T,U>,
    bucket_index : usize
}

impl <T,U>RoutingNode<T,U> {
    fn new(node_info: NodeInfo<T,U>, bucket_index: usize) -> RoutingNode<T,U> {
       RoutingNode {
            node_info : node_info,
            bucket_index : bucket_index,
       }
    }
}

/// The RoutingTable class is used to maintain a list of contacts to which the node is connected.
pub struct RoutingTable<T, U> {
    nodes: Vec<RoutingNode<T,U>>,
    our_name: ::xor_name::XorName,
}

impl <T : PartialEq + HasName + ::std::fmt::Debug + ::std::clone::Clone,
      U : PartialEq + ::std::fmt::Debug + ::std::clone::Clone>RoutingTable<T, U> {
    /// constructor
	pub fn new(our_name: &::xor_name::XorName) -> RoutingTable<T, U> {
        RoutingTable {
            nodes: vec![],
            our_name: our_name.clone(),
        }
    }

    /// Adds a contact to the routing table.  If the contact is added, the first return arg is true,
    /// otherwise false.  If adding the contact caused another contact to be dropped, the dropped
    /// one is returned in the second field, otherwise the optional field is empty.
    pub fn add_node(&mut self, their_info: NodeInfo<T, U>) -> (bool, Option<NodeInfo<T, U>>) {
        if self.our_name == *their_info.name() {
            return (false, None)
        }

        if self.has_node(their_info.name()) {
            debug!("Routing table {:?} has node {:?}. not adding", self.nodes, their_info);
            return (false, None)
        }

        if self.nodes.len() < OPTIMAL_TABLE_SIZE {
            self.push_back_then_sort(their_info);
            return (true, None)
        }

        // we add only if this node is better than any node we currently have
        if let Some(possible_remove) = self.find_candidate_for_removal(&their_info.name()) {
            // self.nodes.retain(|x| x != remove);
            // self.push_back_then_sort(their_info);
            return (true, Some(possible_remove.node_info.clone()));
        } else {
            return (false, None);
        }
    }

    /// Adds a connection to an existing entry.  Should be called after `has_node`. The return
    /// indicates if the given connection was added to an existing NodeInfo.
    pub fn add_connection(&mut self,
                          their_name: &::xor_name::XorName,
                          connection: U) -> bool {
        match self.nodes.iter_mut().find(|node| node.node_info.name() == their_name) {
            Some(mut node) => {
                if node.node_info.connections.iter().any(|elt| *elt == connection) {
                    return false
                }

                node.node_info.connections.push(connection);
                true
            },
            None => {
                error!("The NodeInfo should already exist here.");
                false
            },
        }
    }

    /// This is used to check whether it is worthwhile trying to connect to the peer with a view to
    /// adding the contact to our routing table, i.e. would this contact improve our table.  The
    /// checking procedure is the same as for `add_node`, except for the lack of a public key to
    /// check in step 1.
    pub fn want_to_add(&self, their_name: &::xor_name::XorName) -> bool {
        if self.our_name == *their_name || self.has_node(their_name)  {
            return false
        }

        if self.nodes.len() < OPTIMAL_TABLE_SIZE {
            return true
        }
        let group_len = GROUP_SIZE - 1;
        if ::xor_name::closer_to_target(their_name,
                                        self.nodes[group_len].node_info.name(),
                                        &self.our_name) {
            return true
        }

        if let Some(possible_remove) = self.find_candidate_for_removal(&their_name) {
            true
        } else {
            false
        }
    }

    /// This unconditionally removes the contact from the table.
    pub fn drop_node(&mut self, node_to_drop: &::xor_name::XorName) {
        self.nodes.retain(|node| node.node_info.name() != node_to_drop);
    }

    /// This should be called when a connection has dropped.  If the
    /// affected entry has no connections after removing this one, the entry is removed from the
    /// routing table and its name is returned.  If the entry still has at least one connection, or
    /// an entry cannot be found for 'lost_connection', the function returns 'None'.
    pub fn drop_connection(&mut self, lost_connection: &U) -> Option<::xor_name::XorName> {
        let remove_connection = |node: &mut RoutingNode<T,U>| {
            if let Some(index) = node.node_info.connections
                                          .iter()
                                          .position(|connection| connection == lost_connection) {
                let _ = node.node_info.connections.remove(index);
                true
            } else {
                false
            }
        };
        if let Some(node_index) = self.nodes.iter_mut().position(remove_connection) {
            if self.nodes[node_index].node_info.connections.is_empty() {
               return Some(self.nodes.remove(node_index).node_info.name().clone())
            }
        }
        None
    }

    /// This returns a collection of contacts to which a message should be sent onwards.  It will
    /// return all of our close group (comprising 'GROUP_SIZE' contacts) if the closest one to the
    /// target is within our close group.  If not, it will return either the 'PARALLELISM' closest
    /// contacts to the target or a single contact if 'target' is the name of a contact in the table.
    pub fn target_nodes(&self, target: &::xor_name::XorName) -> Vec<NodeInfo<T, U>> {
        //if in range of close_group send to all close_group
        if self.is_close(target) {
            return self.our_close_group()
        }

        // if not in close group but connected then send direct
        if let Some(ref found) = self.nodes.iter().find(|ref node| node.node_info.name() == target) {
            return vec![(*found).node_info.clone()]
        }

        // not in close group or routing table so send to closest known nodes up to parallelism
        // count
        self.nodes.iter()
               .sorted_by(|a, b| if ::xor_name::closer_to_target(&a.node_info.name(),
                                                                 &b.node_info.name(),
                                                                 &target) {
                                     ::std::cmp::Ordering::Less
                                 } else {
                                     ::std::cmp::Ordering::Greater
                                 })
               .into_iter()
               .cloned()
               .take(PARALLELISM)
               .into_iter()
               .map(|x| x.node_info)
               .collect_vec()
    }

    /// This returns our close group, i.e. the 'GROUP_SIZE' contacts closest to our name (or the
    /// entire table if we hold less than 'GROUP_SIZE' contacts in total) sorted by closeness to us.
    pub fn our_close_group(&self) -> Vec<NodeInfo<T, U>> {
        self.nodes.iter().take(GROUP_SIZE).cloned().map(|x| x.node_info).collect_vec()
    }

    /// This returns true if the provided name is closer than or equal to the furthest node in our
    /// close group. If the routing table contains less than GROUP_SIZE nodes, then every address is
    /// considered to be close.
    pub fn is_close(&self, name: &::xor_name::XorName) -> bool {
        match self.nodes.iter().nth(GROUP_SIZE - 1) {
            Some(node) => ::xor_name::closer_to_target_or_equal(name,
                                                                node.node_info.name(),
                                                                &self.our_name),
            None => true
        }
    }
    /// number of elements
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

	/// empty
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    /// routing table name
    pub fn our_name(&self) -> &::xor_name::XorName {
        &self.our_name
    }
    /// check is routing table contains name
    pub fn has_node(&self, name: &::xor_name::XorName) -> bool {
        self.nodes.iter().any(|node| node.node_info.name() == name)
    }

    fn furthest_close_node(&self) -> Option<&NodeInfo<T, U>> {
        match self.nodes.iter().nth(GROUP_SIZE - 1) {
            Some(node) => Some(&node.node_info),
            None => self.nodes.last().map(|x| &x.node_info)
        }
    }

    // This effectively reverse iterates through all non-empty buckets (i.e. starts at furthest
    // bucket from us) checking for overfilled ones and returning the table index of the furthest
    // contact within that bucket.  No contacts within our close group will be considered.
    fn find_candidate_for_removal(&self, name: &::xor_name::XorName) -> Option<&RoutingNode<T, U>> {
        assert!(self.nodes.len() >= OPTIMAL_TABLE_SIZE);
        let it = self.nodes.iter().rev();
        let their_bucket_index = self.bucket_index(name);
        let count = 0;

        // while *it.node_info.name() != self.furthest_close_node() || it.next() < it.last() {
        //   count += 1;
        //   if *it.bucket_index != their_bucket_index { count = 1; continue; }
        //   if count > BUCKET_SIZE { return *it.name_info.name() }
        // }
    None
    }

    // This is equivalent to the common leading bits of `self.our_name` and `name` where "leading
    // bits" means the most significant bits.
    fn bucket_index(&self, name: &::xor_name::XorName) -> usize {
        self.our_name.bucket_distance(name)
    }

    fn push_back_then_sort(&mut self, node_info: NodeInfo<T, U>) {
        {  // Try to find and update an existing entry
            if let Some(mut entry) = self.nodes
                                         .iter_mut()
                                         .find(|element| element.node_info.name() == node_info.name()) {
                entry.node_info.connections.extend(node_info.connections);
                return
            }
        }
        // We didn't find an existing entry, so insert a new one
        let bucket = self.bucket_index(&node_info.name());
        self.nodes.push(RoutingNode::new(node_info, bucket));
        let our_name = &self.our_name;
        self.nodes.sort_by(
            |lhs, rhs| if ::xor_name::closer_to_target(lhs.node_info.name(),
                                                       rhs.node_info.name(),
                                                       our_name) {
                           ::std::cmp::Ordering::Less
                       } else {
                           ::std::cmp::Ordering::Greater
                       });
    }

}



#[cfg(test)]
mod test {
    extern crate bit_vec;
    use super::{RoutingTable, NodeInfo, GROUP_SIZE, OPTIMAL_TABLE_SIZE, PARALLELISM, QUORUM_SIZE,
	            HasName};

   #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestNodeInfo {
       name : ::xor_name::XorName,
    }

    impl TestNodeInfo {
        fn new() -> TestNodeInfo {
            TestNodeInfo {
            name: ::rand::random::<::xor_name::XorName>()
            }
        }
        fn set_name(&mut self, name: ::xor_name::XorName) {
            self.name = name;
        }
    }

    impl HasName for TestNodeInfo {
        fn name(&self) ->&::xor_name::XorName {
            &self.name
        }
    }
    enum ContactType {
        Far,
        Mid,
        Close,
    }

    struct Bucket {
        far_contact: ::xor_name::XorName,
        mid_contact: ::xor_name::XorName,
        close_contact: ::xor_name::XorName,
    }

    impl Bucket {
        fn new(farthest_from_tables_own_name: ::xor_name::XorName, index: usize) -> Bucket {
            Bucket {
                far_contact: Self::get_contact(&farthest_from_tables_own_name, index,
                                               ContactType::Far),
                mid_contact: Self::get_contact(&farthest_from_tables_own_name, index,
                                               ContactType::Mid),
                close_contact: Self::get_contact(&farthest_from_tables_own_name, index,
                                                 ContactType::Close),
            }
        }
        fn get_contact(farthest_from_tables_own_name: &::xor_name::XorName,
                       index: usize,
                       contact_type: ContactType)
                       -> ::xor_name::XorName {
            let mut binary_name =
                self::bit_vec::BitVec::from_bytes(&farthest_from_tables_own_name.0);
            if index > 0 {
                for i in 0..index {
                    let bit = unwrap_option!(binary_name.get(i), "");
                    binary_name.set(i, !bit);
                }
            }

            match contact_type {
                ContactType::Mid => {
                    let bit_num = binary_name.len() - 1;
                    let bit = unwrap_option!(binary_name.get(bit_num), "");
                    binary_name.set(bit_num, !bit);
                }
                ContactType::Close => {
                    let bit_num = binary_name.len() - 2;
                    let bit = unwrap_option!(binary_name.get(bit_num), "");
                    binary_name.set(bit_num, !bit);
                }
                ContactType::Far => {}
            };

            let mut arr =  [0u8; 64];
			let bytes = binary_name.to_bytes();
			for i in 0..64 {
				arr[i] = bytes[..][i];
			}
            ::xor_name::XorName(arr)
        }

    }

    struct TestEnvironment {
        table: RoutingTable<TestNodeInfo, u64>,
        buckets: Vec<Bucket>,
        node_info: NodeInfo<TestNodeInfo, u64>,
        initial_count: usize,
        added_names: Vec<::xor_name::XorName>,
    }

    impl TestEnvironment {
        fn new() -> TestEnvironment {
            let node_info = create_random_node_info();
            TestEnvironment {
                table: RoutingTable::new(node_info.name()),
                buckets: initialise_buckets(node_info.name()),
                node_info: node_info,
                initial_count: (::rand::random::<usize>() % (GROUP_SIZE - 1)) + 1,
                added_names: Vec::new(),
            }
        }

        fn partially_fill_table(&mut self) {
            for i in 0..self.initial_count {
                self.node_info.public_id.set_name(self.buckets[i].mid_contact);
                self.added_names.push(self.node_info.name().clone());
                assert!(self.table.add_node(self.node_info.clone()).0);
            }

            assert_eq!(self.initial_count, self.table.len());
            assert!(are_nodes_sorted(&self.table), "Nodes are not sorted");
        }

        fn complete_filling_table(&mut self) {
            for i in self.initial_count..OPTIMAL_TABLE_SIZE {
                self.node_info.public_id.set_name(self.buckets[i].mid_contact);
                self.added_names.push(self.node_info.name().clone());
                assert!(self.table.add_node(self.node_info.clone()).0);
            }

            assert_eq!(OPTIMAL_TABLE_SIZE, self.table.len());
            assert!(are_nodes_sorted(&self.table), "Nodes are not sorted");
        }

        fn public_id(&self, name: &::xor_name::XorName) -> Option<TestNodeInfo> {
            assert!(are_nodes_sorted(&self.table), "Nodes are not sorted");
            match self.table.nodes.iter().find(|node_info| node_info.name() == name) {
                Some(node) => Some(node.public_id.clone()),
                None => None,
            }
        }
    }

    fn initialise_buckets(name: &::xor_name::XorName) -> Vec<Bucket> {
        let arr = [255u8; 64];
        let mut arr_res = [0u8; 64];
        for i in 0..64 {
            arr_res[i] = arr[i] ^ name.0[i];
        }

        let farthest_from_tables_own_name = ::xor_name::XorName::new(arr_res);

        let mut buckets = Vec::new();
        for i in 0..100 {
            buckets.push(Bucket::new(farthest_from_tables_own_name.clone(), i));
            assert!(::xor_name::closer_to_target(&buckets[i].mid_contact,
                                                  &buckets[i].far_contact,
                                                  name));
            assert!(::xor_name::closer_to_target(&buckets[i].close_contact,
                                                  &buckets[i].mid_contact,
                                                  name));
            if i != 0 {
                assert!(::xor_name::closer_to_target(&buckets[i].far_contact,
                                                      &buckets[i - 1].close_contact,
                                                      name));
            }
        }
        buckets
    }

    fn create_random_node_info() -> NodeInfo<TestNodeInfo, u64> {
        NodeInfo {
            public_id: TestNodeInfo::new(),
            connections: Vec::new(),
        }
    }

    fn create_random_routing_tables(num_of_tables: usize) -> Vec<RoutingTable<TestNodeInfo, u64>> {
        use rand;
        let mut vector: Vec<RoutingTable<TestNodeInfo, u64>> = Vec::with_capacity(num_of_tables);
        for _ in 0..num_of_tables {
            vector.push(RoutingTable::new(&rand::random()));
        }
        vector
    }

    fn are_nodes_sorted(routing_table: &RoutingTable<TestNodeInfo, u64>) -> bool {
        if routing_table.nodes.len() < 2 {
            true
        } else {
            routing_table.nodes.windows(2).all(|window|
                ::xor_name::closer_to_target(window[0].name(), window[1].name(),
                                              &routing_table.our_name))
        }
    }

    fn make_sort_predicate(target: ::xor_name::XorName)
                           -> Box<FnMut(&::xor_name::XorName, &::xor_name::XorName) -> ::std::cmp::Ordering> {
        Box::new(move |lhs: &::xor_name::XorName, rhs: &::xor_name::XorName| {
            match ::xor_name::closer_to_target(lhs, rhs, &target) {
                true => ::std::cmp::Ordering::Less,
                false => ::std::cmp::Ordering::Greater,
            }
        })
    }

    #[test]
    fn add_node() {
        let mut test = TestEnvironment::new();

        assert_eq!(test.table.len(), 0);

        // try with our name - should fail
        test.node_info.public_id.set_name(test.table.our_name);
        assert_eq!((false, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(test.table.len(), 0);

        // add first contact
        test.node_info.public_id.set_name(test.buckets[0].far_contact);
        assert_eq!((true, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(test.table.len(), 1);

        // try with the same contact - should fail
        assert_eq!((false, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(test.table.len(), 1);

        // Add further 'OPTIMAL_TABLE_SIZE' - 1 contacts (should all succeed with no removals).  Set
        // this up so that bucket 0 (furthest) and bucket 1 have 3 contacts each and all others have
        // 0 or 1 contacts.

        // Bucket 0
        test.node_info.public_id.set_name(test.buckets[0].mid_contact);
        assert_eq!((true, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(2, test.table.len());
        assert_eq!((false, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(2, test.table.len());

        test.node_info.public_id.set_name(test.buckets[0].close_contact);
        assert_eq!((true, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(3, test.table.len());
        assert_eq!((false, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(3, test.table.len());

        // Bucket 1
        test.node_info.public_id.set_name(test.buckets[1].far_contact);
        assert_eq!((true, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(4, test.table.len());
        assert_eq!((false, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(4, test.table.len());

        test.node_info.public_id.set_name(test.buckets[1].mid_contact);
        assert_eq!((true, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(5, test.table.len());
        assert_eq!((false, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(5, test.table.len());

        test.node_info.public_id.set_name(test.buckets[1].close_contact);
        assert_eq!((true, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(6, test.table.len());
        assert_eq!((false, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(6, test.table.len());

        // Add remaining contacts
        for i in 2..(OPTIMAL_TABLE_SIZE - 4) {
            test.node_info.public_id.set_name(test.buckets[i].mid_contact);
            assert_eq!((true, None), test.table.add_node(test.node_info.clone()));
            assert_eq!(i + 5, test.table.len());
            assert_eq!((false, None), test.table.add_node(test.node_info.clone()));
            assert_eq!(i + 5, test.table.len());
        }

        // Check next 4 closer additions return 'buckets_[0].far_contact',
        // 'buckets_[0].mid_contact', 'buckets_[1].far_contact', and 'buckets_[1].mid_contact' as
        // dropped (in that order)
        let mut dropped: Vec<::xor_name::XorName> = Vec::new();
        let optimal_len = OPTIMAL_TABLE_SIZE;
        for i in (optimal_len - 4)..optimal_len {
            test.node_info.public_id.set_name(test.buckets[i].mid_contact);
            let result_of_add = test.table.add_node(test.node_info.clone());
            assert!(result_of_add.0);
            dropped.push(unwrap_option!(result_of_add.1, "").name().clone());
            assert_eq!(OPTIMAL_TABLE_SIZE, test.table.len());
            assert_eq!((false, None), test.table.add_node(test.node_info.clone()));
            assert_eq!(OPTIMAL_TABLE_SIZE, test.table.len());
        }
        assert!(test.buckets[0].far_contact == dropped[0]);
        assert!(test.buckets[0].mid_contact == dropped[1]);
        assert!(test.buckets[1].far_contact == dropped[2]);
        assert!(test.buckets[1].mid_contact == dropped[3]);

        // Try to add far contacts again (should fail)
        for far_contact in dropped {
            test.node_info.public_id.set_name(far_contact);
            assert_eq!((false, None), test.table.add_node(test.node_info.clone()));
            assert_eq!(OPTIMAL_TABLE_SIZE, test.table.len());
        }

        // Add final close contact to push len() of table above OPTIMAL_TABLE_SIZE
        test.node_info.public_id.set_name(test.buckets[OPTIMAL_TABLE_SIZE].mid_contact);
        assert_eq!((true, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(OPTIMAL_TABLE_SIZE + 1, test.table.len());
        assert_eq!((false, None), test.table.add_node(test.node_info.clone()));
        assert_eq!(OPTIMAL_TABLE_SIZE + 1, test.table.len());
    }

    #[test]
    fn add_connection() {
                                                                                                            // implement
    }

    #[test]
    fn want_to_add() {
        let mut test = TestEnvironment::new();

        // Try with our ID
        assert!(!test.table.want_to_add(&test.table.our_name));

        // Should return true for empty routing table
        assert!(test.table.want_to_add(&test.buckets[0].far_contact));

        // Add the first contact, and check it doesn't allow duplicates
        let mut new_node_0 = create_random_node_info();
        new_node_0.public_id.set_name(test.buckets[0].far_contact);
        assert!(test.table.add_node(new_node_0).0);
        assert!(!test.table.want_to_add(&test.buckets[0].far_contact));

        // Add further 'OPTIMAL_TABLE_SIZE' - 1 contact (should all succeed with no removals).  Set
        // this up so that bucket 0 (furthest) and bucket 1 have 3 contacts each and all others have
        // 0 or 1 contacts.

        let mut new_node_1 = create_random_node_info();
        new_node_1.public_id.set_name(test.buckets[0].mid_contact);
        assert!(test.table.want_to_add(new_node_1.name()));
        assert!(test.table.add_node(new_node_1).0);
        assert!(!test.table.want_to_add(&test.buckets[0].mid_contact));

        let mut new_node_2 = create_random_node_info();
        new_node_2.public_id.set_name(test.buckets[0].close_contact);
        assert!(test.table.want_to_add(new_node_2.name()));
        assert!(test.table.add_node(new_node_2).0);
        assert!(!test.table.want_to_add(&test.buckets[0].close_contact));

        let mut new_node_3 = create_random_node_info();
        new_node_3.public_id.set_name(test.buckets[1].far_contact);
        assert!(test.table.want_to_add(new_node_3.name()));
        assert!(test.table.add_node(new_node_3).0);
        assert!(!test.table.want_to_add(&test.buckets[1].far_contact));

        let mut new_node_4 = create_random_node_info();
        new_node_4.public_id.set_name(test.buckets[1].mid_contact);
        assert!(test.table.want_to_add(new_node_4.name()));
        assert!(test.table.add_node(new_node_4).0);
        assert!(!test.table.want_to_add(&test.buckets[1].mid_contact));

        let mut new_node_5 = create_random_node_info();
        new_node_5.public_id.set_name(test.buckets[1].close_contact);
        assert!(test.table.want_to_add(new_node_5.name()));
        assert!(test.table.add_node(new_node_5).0);
        assert!(!test.table.want_to_add(&test.buckets[1].close_contact));

        for i in 2..(OPTIMAL_TABLE_SIZE - 4) {
            let mut new_node = create_random_node_info();
            new_node.public_id.set_name(test.buckets[i].mid_contact);
            assert!(test.table.want_to_add(new_node.name()));
            assert!(test.table.add_node(new_node).0);
            assert!(!test.table.want_to_add(&test.buckets[i].mid_contact));
        }

        assert_eq!(OPTIMAL_TABLE_SIZE, test.table.nodes.len());

        let optimal_len = OPTIMAL_TABLE_SIZE;
        for i in (optimal_len - 4)..optimal_len {
            let mut new_node = create_random_node_info();
            new_node.public_id.set_name(test.buckets[i].mid_contact);
            assert!(test.table.want_to_add(new_node.name()));
            assert!(test.table.add_node(new_node).0);
            assert!(!test.table.want_to_add(&test.buckets[i].mid_contact));
            assert_eq!(OPTIMAL_TABLE_SIZE, test.table.nodes.len());
        }

        // Check for contacts again which are now not in the table
        assert!(!test.table.want_to_add(&test.buckets[0].far_contact));
        assert!(!test.table.want_to_add(&test.buckets[0].mid_contact));
        assert!(!test.table.want_to_add(&test.buckets[1].far_contact));
        assert!(!test.table.want_to_add(&test.buckets[1].mid_contact));

        // Check final close contact which would push len() of table above OPTIMAL_TABLE_SIZE
        assert!(test.table.want_to_add(&test.buckets[OPTIMAL_TABLE_SIZE].mid_contact));
    }

    #[test]
    fn drop_node() {
        use ::rand::Rng;

        // Check on empty table
        let mut test = TestEnvironment::new();

        assert_eq!(test.table.len(), 0);

        // Fill the table
        test.partially_fill_table();
        test.complete_filling_table();

        // Try with invalid Address
        test.table.drop_node(&::xor_name::XorName::new([0u8; 64]));
        assert_eq!(OPTIMAL_TABLE_SIZE, test.table.len());

        // Try with our Name
        let drop_name = test.table.our_name.clone();
        test.table.drop_node(&drop_name);
        assert_eq!(OPTIMAL_TABLE_SIZE, test.table.len());

        // Try with Address of node not in table
        test.table.drop_node(&test.buckets[0].far_contact);
        assert_eq!(OPTIMAL_TABLE_SIZE, test.table.len());

        // Remove all nodes one at a time in random order
        let mut rng = ::rand::thread_rng();
        rng.shuffle(&mut test.added_names[..]);
        let mut len = test.table.len();
        for name in test.added_names {
            len -= 1;
            test.table.drop_node(&name);
            assert_eq!(len, test.table.len());
        }
    }

    #[test]
    fn drop_connection() {
                                                                                                            // implement
    }

    #[test]
    fn target_nodes() {
                                                                                                            // modernise
        use rand;
        let mut test = TestEnvironment::new();

        // Check on empty table
        let mut target_nodes = test.table.target_nodes(&rand::random());
        assert_eq!(target_nodes.len(), 0);

        // Partially fill the table with <GROUP_SIZE contacts
        test.partially_fill_table();

        // Check we get all contacts returned
        target_nodes = test.table.target_nodes(&rand::random());
        assert_eq!(test.initial_count, target_nodes.len());

        for i in 0..test.initial_count {
            let mut assert_checker = 0;
            for j in 0..target_nodes.len() {
                if *target_nodes[j].name() == test.buckets[i].mid_contact {
                    assert_checker = 1;
                    break;
                }
            }
            assert!(assert_checker == 1);
        }

        // Complete filling the table up to OPTIMAL_TABLE_SIZE contacts
        test.complete_filling_table();

        // Try with our ID (should return closest to us, i.e. buckets 63 to 32)
        target_nodes = test.table.target_nodes(&test.table.our_name);
        assert_eq!(GROUP_SIZE, target_nodes.len());

        for i in ((OPTIMAL_TABLE_SIZE -
                   GROUP_SIZE)..
                   OPTIMAL_TABLE_SIZE - 1).rev() {
            let mut assert_checker = 0;
            for j in 0..target_nodes.len() {
                if *target_nodes[j].name() == test.buckets[i].mid_contact {
                    assert_checker = 1;
                    break;
                }
            }
            assert!(assert_checker == 1);
        }

        // Try with nodes far from us, first time *not* in table and second time *in* table (should
        // return 'PARALLELISM' contacts closest to target first time and the single actual target
        // the second time)
        let mut target: ::xor_name::XorName;
        for count in 0..2 {
            for i in 0..(OPTIMAL_TABLE_SIZE -
                         GROUP_SIZE) {
                let (target, expected_len) = if count == 0 {
                    (test.buckets[i].far_contact.clone(), PARALLELISM)
                } else {
                    (test.buckets[i].mid_contact.clone(), 1)
                };
                target_nodes = test.table.target_nodes(&target);
                assert_eq!(expected_len, target_nodes.len());
                for i in 0..target_nodes.len() {
                    let mut assert_checker = 0;
                    for j in 0..test.added_names.len() {
                        if *target_nodes[i].name() == test.added_names[j] {
                            assert_checker = 1;
                            continue;
                        }
                    }
                    assert!(assert_checker == 1);
                }
            }
        }

        // Try with nodes close to us, first time *not* in table and second time *in* table (should
        // return GROUP_SIZE closest to target)
        for count in 0..2 {
            for i in (OPTIMAL_TABLE_SIZE -
                      GROUP_SIZE)..
                      OPTIMAL_TABLE_SIZE {
                target = if count == 0 {
                    test.buckets[i].close_contact.clone()
                } else {
                    test.buckets[i].mid_contact.clone()
                };
                target_nodes = test.table.target_nodes(&target);
                assert_eq!(GROUP_SIZE, target_nodes.len());
                for i in 0..target_nodes.len() {
                    let mut assert_checker = 0;
                    for j in 0..test.added_names.len() {
                        if *target_nodes[i].name() == test.added_names[j] {
                            assert_checker = 1;
                            continue;
                        }
                    }
                    assert!(assert_checker == 1);
                }
            }
        }
    }


    #[test]
    fn our_close_group_test() {
                                                                                                    // unchecked - could be merged with one below?
        let mut test = TestEnvironment::new();
        assert!(test.table.our_close_group().is_empty());

        test.partially_fill_table();
        assert_eq!(test.initial_count, test.table.our_close_group().len());

        for i in 0..test.initial_count {
            assert!(test.table
                        .our_close_group()
                        .iter()
                        .filter(|node| *node.name() == test.buckets[i].mid_contact)
                        .count()
                        > 0);
        }

        test.complete_filling_table();
        assert_eq!(GROUP_SIZE, test.table.our_close_group().len());

        for close_node in &test.table.our_close_group() {
            assert_eq!(1, test.added_names.iter().filter(|n| *n == close_node.name()).count());
        }
    }

    #[test]
    fn our_close_group_and_is_close() {
                                                                                                    // unchecked - could be merged with one above?
        // independent double verification of our_close_group()
        // this test verifies that the close group is returned sorted
        let name = ::rand::random::<::xor_name::XorName>();
        let mut routing_table = RoutingTable::new(&name);

        let mut count: usize = 0;
        loop {
            let _ = routing_table.add_node(NodeInfo::new(
                TestNodeInfo::new(), vec![]));
            count += 1;
            if routing_table.len() >= OPTIMAL_TABLE_SIZE {
                break;
            }
            if count >= 2 * OPTIMAL_TABLE_SIZE {
                panic!("Routing table does not fill up.");
            }
        }
        let our_close_group: Vec<NodeInfo<TestNodeInfo, u64>> = routing_table.our_close_group();
        assert_eq!(our_close_group.len(), GROUP_SIZE);
        let mut closer_name: ::xor_name::XorName = name.clone();
        for close_node in &our_close_group {
            assert!(::xor_name::closer_to_target(&closer_name, close_node.name(), &name));
            assert!(routing_table.is_close(close_node.name()));
            closer_name = close_node.name().clone();
        }
        for node in &routing_table.nodes {
            if our_close_group.iter()
                              .filter(|close_node| close_node.name() == node.name())
                              .count() > 0 {
                assert!(routing_table.is_close(node.name()));
            } else {
                assert!(!routing_table.is_close(node.name()));
            }
        }
    }

    #[test]
    fn add_check_close_group_test() {
                                                                                                    // unchecked - could be merged with one above?
        let num_of_tables = 50usize;
        let mut tables = create_random_routing_tables(num_of_tables);
        let mut addresses: Vec<::xor_name::XorName> = Vec::with_capacity(num_of_tables);

        for i in 0..num_of_tables {
            addresses.push(tables[i].our_name.clone());
            for j in 0..num_of_tables {
                let mut node_info = create_random_node_info();
                node_info.public_id.set_name(tables[j].our_name);
                let _ = tables[i].add_node(node_info);
            }
        }
        for it in tables.iter() {
            addresses.sort_by(&mut *make_sort_predicate(it.our_name.clone()));
            let mut groups = it.our_close_group();
            assert_eq!(groups.len(), GROUP_SIZE);

            // TODO(Spandan) vec.dedup does not compile - manually doing it
            if groups.len() > 1 {
                let mut new_end = 1usize;
                for i in 1..groups.len() {
                    if groups[new_end - 1].name() != groups[i].name() {
                        if new_end != i {
                            groups[new_end] = groups[i].clone();
                        }
                        new_end += 1;
                    }
                }
                assert_eq!(new_end, groups.len());
            }

            assert_eq!(groups.len(), GROUP_SIZE);

            for i in 0..GROUP_SIZE {
                assert!(groups[i].name() == &addresses[i + 1]);
            }
        }
    }

    #[test]
    fn churn_test() {
                                                                                                    // unchecked - purpose?
        let network_len = 200usize;
        let nodes_to_remove = 20usize;

        let mut tables = create_random_routing_tables(network_len);
        let mut addresses: Vec<::xor_name::XorName> = Vec::with_capacity(network_len);

        for i in 0..tables.len() {
            addresses.push(tables[i].our_name.clone());
            for j in 0..tables.len() {
                let mut node_info = create_random_node_info();
                node_info.public_id.set_name(tables[j].our_name);
                let _ = tables[i].add_node(node_info);
            }
        }

        // now remove nodes
        let mut drop_vec: Vec<::xor_name::XorName> = Vec::with_capacity(nodes_to_remove);
        for i in 0..nodes_to_remove {
            drop_vec.push(addresses[i].clone());
        }

        tables.truncate(nodes_to_remove);

        for i in 0..tables.len() {
            for j in 0..drop_vec.len() {
                tables[i].drop_node(&drop_vec[j]);
            }
        }
        // remove IDs too
        addresses.truncate(nodes_to_remove);

        for i in 0..tables.len() {
            addresses.sort_by(&mut *make_sort_predicate(tables[i].our_name.clone()));
            let group = tables[i].our_close_group();
            assert_eq!(group.len(), ::std::cmp::min(GROUP_SIZE, tables[i].len()));
        }
    }

    #[test]
    fn target_nodes_group_test() {
                                                                                                    // unchecked - purpose?
        let network_len = 100usize;

        let mut tables = create_random_routing_tables(network_len);
        let mut addresses: Vec<::xor_name::XorName> = Vec::with_capacity(network_len);

        for i in 0..tables.len() {
            addresses.push(tables[i].our_name.clone());
            for j in 0..tables.len() {
                let mut node_info = create_random_node_info();
                node_info.public_id.set_name(tables[j].our_name);
                let _ = tables[i].add_node(node_info);
            }
        }

        for i in 0..tables.len() {
            addresses.sort_by(&mut *make_sort_predicate(tables[i].our_name.clone()));
            // if target is in close group return the whole close group excluding target
            for j in 1..(GROUP_SIZE - QUORUM_SIZE) {
                let target_close_group = tables[i].target_nodes(&addresses[j]);
                assert_eq!(GROUP_SIZE, target_close_group.len());
                // should contain our close group
                for k in 0..target_close_group.len() {
                    assert_eq!(*target_close_group[k].name(), addresses[k + 1]);
                }
            }
        }
    }

    #[test]
    fn trivial_functions_test() {
                                                                                            // unchecked - but also check has_node function
        let mut test = TestEnvironment::new();
        assert!(test.public_id(&test.buckets[0].mid_contact).is_none());
        assert_eq!(0, test.table.nodes.len());

        // Check on partially filled the table
        test.partially_fill_table();
        let test_node = create_random_node_info();
        test.node_info = test_node.clone();
        assert!(test.table.add_node(test.node_info.clone()).0);

        match test.public_id(test.node_info.name()) {
            Some(_) => {}
            None => panic!("PublicId None"),
        }
        // EXPECT_TRUE(asymm::MatchingKeys(info_.dht_public_id.public_key(),
        //                                 *table_.GetPublicKey(info_.name())));
        match test.public_id(&test.buckets[test.buckets.len() - 1].far_contact) {
            Some(_) => panic!("PublicId Exits"),
            None => {}
        }
        assert_eq!(test.initial_count + 1, test.table.nodes.len());

        // Check on fully filled the table
        test.table.drop_node(test_node.name());
        test.complete_filling_table();
        test.table.drop_node(&test.buckets[0].mid_contact);
        test.node_info = test_node.clone();
        assert!(test.table.add_node(test.node_info.clone()).0);

        match test.public_id(test.node_info.name()) {
            Some(_) => {}
            None => panic!("PublicId None"),
        }
        match test.public_id(&test.buckets[test.buckets.len() - 1].far_contact) {
            Some(_) => panic!("PublicId Exits"),
            None => {}
        }
        // EXPECT_TRUE(asymm::MatchingKeys(info_.dht_public_id.public_key(),
        //                                 *table_.GetPublicKey(info_.name())));
        assert_eq!(OPTIMAL_TABLE_SIZE, test.table.nodes.len());
    }

    #[test]
    fn bucket_index() {
        // Set our name for routing table to max possible value (in binary, all `1`s)
        let our_name = ::xor_name::XorName::new([255u8; ::xor_name::XOR_NAME_LEN]);
        let routing_table = RoutingTable::<TestNodeInfo, u64>::new(&our_name);

        // Iterate through each u8 element of a target name identical to ours and set it to each
        // possible value for u8 other than 255 (since that which would a target name identical to
        // our name)
        for index in 0..::xor_name::XOR_NAME_LEN {
            let mut array = [255u8; ::xor_name::XOR_NAME_LEN];
            for modified_element in 0..255u8 {
                array[index] = modified_element;
                let target_name = ::xor_name::XorName::new(array);
                // `index` is equivalent to common leading bytes, so the common leading bits (CLBs)
                // is `index` * 8 plus some value for `modified_element`.  Where
                // 0 <= modified_element < 128, the first bit is different so CLBs is 0, and for
                // 128 <= modified_element < 192, the second bit is different, so CLBs is 1, and so
                // on.
                let expected_bucket_index = (index * 8) + match modified_element {
                    0...127 => 0,
                    128...191 => 1,
                    192...223 => 2,
                    224...239 => 3,
                    240...247 => 4,
                    248...251 => 5,
                    252 | 253 => 6,
                    254 => 7,
                    _ => unreachable!(),
                };
                if expected_bucket_index != routing_table.bucket_index(&target_name) {
                    let as_binary = |name: &::xor_name::XorName| -> String {
                        let mut name_as_binary = String::new();
                        for i in name.0.iter() {
                            name_as_binary.push_str(&format!("{:08b}", i));
                        }
                        name_as_binary
                    };
                    println!("us:   {}", as_binary(&our_name));
                    println!("them: {}", as_binary(&target_name));
                    println!("index:                 {}", index);
                    println!("modified_element:      {}", modified_element);
                    println!("expected bucket_index: {}", expected_bucket_index);
                    println!("actual bucket_index:   {}", routing_table.bucket_index(&target_name));
                }
                assert_eq!(expected_bucket_index, routing_table.bucket_index(&target_name));
            }
        }

        // Check the bucket index of our own name is 512
        assert_eq!(::xor_name::XOR_NAME_LEN * 8, routing_table.bucket_index(&our_name));
    }
}
