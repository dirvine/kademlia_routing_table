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

// #![allow(unused)]

extern crate kademlia_routing_table;
extern crate rand;
extern crate xor_name;

use kademlia_routing_table::{Destination, RoutingTable, GROUP_SIZE, PARALLELISM};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering, ATOMIC_USIZE_INIT};
use xor_name::XorName;

// Simulated network endpoint. In the real networks, this would be something
// like ip address and port pair.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Endpoint(usize);

// Simulated connection to an endpoint.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Connection(Endpoint);

trait Authority {
    fn name(&self) -> &XorName;
    fn is_group(&self) -> bool;
}

impl Authority for Destination {
    fn name(&self) -> &XorName {
        match *self {
            Destination::Group(ref name) => name,
            Destination::Node(ref name) => name,
        }
    }

    fn is_group(&self) -> bool {
        match *self {
            Destination::Group(_) => true,
            Destination::Node(_) => false,
        }
    }
}

type MessageId = usize;

// This is used to generate unique message ids.
static mut MESSAGE_ID_COUNTER: AtomicUsize = ATOMIC_USIZE_INIT;

#[allow(unsafe_code)]
fn next_message_id() -> MessageId {
    unsafe { MESSAGE_ID_COUNTER.fetch_add(1, Ordering::Relaxed) }
}

#[derive(Clone, Debug)]
struct Message {
    id: MessageId,
    src: Destination,
    dst: Destination,
    // Names of all the nodes the message passed through.
    route: Vec<XorName>,
}

impl Message {
    fn new(src: Destination, dst: Destination) -> Self {
        Message {
            id: next_message_id(),
            src: src,
            dst: dst,
            route: Vec::new(),
        }
    }

    fn hop_name(&self) -> &XorName {
        self.route.last().unwrap()
    }
}

// Records how many times a particular message was received and/or sent by a
// node.
struct MessageStats(HashMap<MessageId, (usize, usize)>);

impl MessageStats {
    fn new() -> Self {
        MessageStats(HashMap::new())
    }

    fn add_received(&mut self, id: MessageId) -> usize {
        let entry = self.entry_mut(id);
        entry.0 += 1;
        entry.0 - 1
    }

    fn add_sent(&mut self, id: MessageId) -> usize {
        let entry = self.entry_mut(id);
        entry.1 += 1;
        entry.1 - 1
    }

    fn get_received(&self, id: MessageId) -> usize {
        self.entry(id).0
    }

    fn get_sent(&self, id: MessageId) -> usize {
        self.entry(id).1
    }

    fn entry(&self, id: MessageId) -> (usize, usize) {
        self.0.get(&id).cloned().unwrap_or((0, 0))
    }

    fn entry_mut(&mut self, id: MessageId) -> &mut (usize, usize) {
        self.0.entry(id).or_insert((0, 0))
    }
}

// Action performed on the network.
enum Action {
    // Send a message via the connection.
    Send(Connection, Message),
}

// Simulated node.
// The nodes can only interact with the network indirectly, by returning lists
// of Actions, so we are sure a node doesn't do anything it wouldn't be able
// to do in the real world.
struct Node {
    name: XorName,
    endpoint: Endpoint,
    table: RoutingTable,
    connections: HashMap<XorName, Connection>,
    message_stats: MessageStats,
    inbox: HashMap<MessageId, Message>,
}

impl Node {
    fn new(name: XorName, endpoint: Endpoint) -> Self {
        let table = RoutingTable::new(&name);

        Node {
            name: name,
            endpoint: endpoint,
            table: table,
            connections: HashMap::new(),
            message_stats: MessageStats::new(),
            inbox: HashMap::new(),
        }
    }

    fn is_connected_to(&self, name: &XorName) -> bool {
        self.table.contains(name)
    }

    fn is_close(&self, name: &XorName) -> bool {
        self.table.is_close(name)
    }

    // Connect to the node with the given name and endpoint.
    // This adds the new node to our routing table.
    fn connect(&mut self, their_name: &XorName, their_endpoint: Endpoint) {
        let _ = self.table.add(their_name.clone());
        let _ = self.connections.insert(their_name.clone(), Connection(their_endpoint));
    }

    fn send_message(&mut self, mut message: Message, handle: bool) -> Vec<Action> {
        let mut actions = Vec::new();

        message.route.push(self.name.clone());

        let count = self.message_stats.get_received(message.id).saturating_sub(1);
        let targets = self.table.target_nodes(message.dst.clone(),
                                              message.hop_name(),
                                              count);

        for target in targets {
            if let Some(&connection) = self.connections.get(&target) {
                actions.push(Action::Send(connection, message.clone()));
                let _ = self.message_stats.add_sent(message.id);
            }
        }

        // Handle the message ourselves if we need to.
        if handle && self.table.is_recipient(message.dst.clone()) &&
           self.message_stats.add_received(message.id) == 0 {
            actions.append(&mut self.on_message(message, false));
        }

        actions
    }

    fn on_message(&mut self, message: Message, relay: bool) -> Vec<Action> {
        let mut actions = Vec::new();

        self.check_direction(&message);

        if self.message_stats.add_received(message.id) > PARALLELISM {
            return actions;
        }

        if relay {
            actions.append(&mut self.send_message(message.clone(), false));
        }

        if self.table.is_recipient(message.dst.clone()) {
            let _ = self.inbox.insert(message.id, message);
        }

        actions
    }

    fn check_direction(&self, message: &Message) {
        if !self.is_swarm(&message.dst, message.hop_name()) {
            if xor_name::closer_to_target(message.hop_name(), &self.name, message.dst.name()) {
                panic!("Direction check failed {:?}", message);
            }
        }
    }

    fn is_swarm(&self, dst: &Destination, hop_name: &XorName) -> bool {
        dst.is_group() &&
        match self.table.other_close_nodes(dst.name()) {
            None => false,
            Some(close_group) => close_group.into_iter().any(|n| n == *hop_name),
        }
    }
}

// Handle to node.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NodeHandle(usize);

// Simulated network. This struct tries to simulate real-world network consisting
// of many nodes. It should expose only operations that would be possible to
// execute on a real network. For example, nodes cannot access other nodes in
// any other means than sending them messages via this network. So nodes don't
// know public endpoints of other nodes unless they have them in their routing
// tables. Operations on the network (for example sending a message to an endpoint)
// are simulated by returning a list of Actions, which the network then executes.
// This is to make sure nodes are only doing what they would be able to do in
// the real world.
struct Network {
    nodes: Vec<Node>,
}

impl Network {
    fn new() -> Self {
        Network {
            nodes: Vec::new(),
        }
    }

    // Add a node with randomly generated name to the network. Returns a
    // node handle which can be used to perform operations with the node.
    fn add_node(&mut self) -> NodeHandle {
        let index = self.nodes.len();
        let name = rand::random();
        let node = Node::new(name, Endpoint(index));

        self.nodes.push(node);

        NodeHandle(index)
    }

    fn get_all_nodes(&self) -> Vec<NodeHandle> {
        (0..self.nodes.len()).map(|i| NodeHandle(i)).collect()
    }

    fn get_random_node(&self) -> NodeHandle {
        self.get_random_nodes(1)[0]
    }

    fn get_two_random_nodes(&self) -> (NodeHandle, NodeHandle) {
        let nodes = self.get_random_nodes(2);
        (nodes[0], nodes[1])
    }

    // Get all nodes that believe they are close to the given name.
    fn get_nodes_close_to(&self, name: &XorName) -> Vec<NodeHandle> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|&(_, node)| node.is_close(name))
            .map(|(index, _)| NodeHandle(index))
            .collect()
    }

    // Get the name and endpoint of a node.
    #[allow(unused)]
    fn get_node_info(&self, handle: NodeHandle) -> (XorName, Endpoint) {
        let node = self.get_node_ref(handle);
        (node.name.clone(), node.endpoint)
    }

    fn get_node_name(&self, handle: NodeHandle) -> XorName {
        self.get_node_ref(handle).name.clone()
    }

    // After this method is called, each nodes will have its routing table
    // fully populated (the kademlia invariant will be satisfied)
    fn bootstrap_all_nodes(&mut self) {
        // Note we are just inserting everyone into everyone elses routing table
        // (if needed/allowed) and skipping the bootstrapping process entirelly.
        //
        // We might need to simulate the bootstrapping in more detail at some
        // point.

        for i in 0..self.nodes.len() {
            let (node0, rest) = self.nodes[i..].split_first_mut().unwrap();

            for node1 in rest {
                Self::connect_if_allowed(node0, node1);
                Self::connect_if_allowed(node1, node0);
            }
        }
    }

    fn connect_if_allowed(node0: &mut Node, node1: &mut Node) {
        if node0.table.need_to_add(&node1.name) && node1.table.allow_connection(&node0.name) {
            node0.connect(&node1.name, node1.endpoint);
            node1.connect(&node0.name, node0.endpoint);
        }
    }

    // Send a message from the node.
    fn send_message(&mut self, node_handle: NodeHandle, message: Message) {
        let actions = self.get_node_mut_ref(node_handle).send_message(message, true);
        self.execute(actions);
    }

    fn is_node_connected_to(&self, node0: NodeHandle, node1: NodeHandle) -> bool {
        let node0 = self.get_node_ref(node0);
        let node1 = self.get_node_ref(node1);

        node0.is_connected_to(&node1.name)
    }

    fn get_contact_count(&self, node: NodeHandle) -> usize {
        self.get_node_ref(node).table.len()
    }

    // Did the node receive a message with the id?
    fn has_node_message_in_inbox(&self, node: NodeHandle, message_id: MessageId) -> bool {
        self.get_node_ref(node).inbox.contains_key(&message_id)
    }

    #[allow(unused)]
    fn get_message_from_inbox(&self, node: NodeHandle, message_id: MessageId) -> Option<&Message> {
        self.get_node_ref(node).inbox.get(&message_id)
    }

    fn get_message_stats(&self, node: NodeHandle) -> &MessageStats {
        &self.get_node_ref(node).message_stats
    }

    // --------------------------------------------------------------------------
    // The following methods are INTERNAL and should not be called in tests.
    // --------------------------------------------------------------------------

    fn get_random_nodes(&self, count: usize) -> Vec<NodeHandle> {
        let mut rng = rand::thread_rng();
        let handles = (0..self.nodes.len()).map(|i| NodeHandle(i));
        rand::sample(&mut rng, handles, count)
    }

    fn get_node_ref(&self, handle: NodeHandle) -> &Node {
        &self.nodes[handle.0]
    }

    fn get_node_mut_ref(&mut self, handle: NodeHandle) -> &mut Node {
        &mut self.nodes[handle.0]
    }

    fn get_node_mut_ref_by_endpoint(&mut self, endpoint: Endpoint) -> &mut Node {
        &mut self.nodes[endpoint.0]
    }

    // Execute list of network actions on this network.
    fn execute(&mut self, actions: Vec<Action>) {
        let mut queue = VecDeque::with_capacity(actions.len());

        for action in actions {
            queue.push_back(action);
        }

        while let Some(action) = queue.pop_front() {
            let new_actions = match action {
                Action::Send(connection, message) => {
                    let node = self.get_node_mut_ref_by_endpoint(connection.0);
                    node.on_message(message, true)
                }
            };

            for new_action in new_actions {
                queue.push_back(new_action);
            }
        }
    }

    #[allow(unused)]
    fn print_stats(&self) {
        println!("");
        println!("=== Network stats ===");

        for node in &self.nodes {
            println!("{:?}: {} contacts", node.endpoint, node.table.len());
        }
    }
}

fn create_network(nodes_count: usize) -> Network {
    let mut network = Network::new();

    for _ in 0..nodes_count {
        let _ = network.add_node();
    }

    network.bootstrap_all_nodes();

    println!("");
    println!("Num nodes: {}", nodes_count);
    println!("");

    network
}

const NODES_COUNT: usize = 32;
const SAMPLES: usize = 100;

#[test]
fn number_of_nodes_close_to_any_name_is_equal_to_group_size() {
    let network = create_network(NODES_COUNT);

    for _ in 0..SAMPLES {
        let name = rand::random();
        assert_eq!(network.get_nodes_close_to(&name).len(), GROUP_SIZE);
    }
}

#[test]
fn node_is_connected_to_every_node_in_its_close_group() {
    let network = create_network(NODES_COUNT);

    for _ in 0..SAMPLES {
        let node = network.get_node_name(network.get_random_node());
        let close_group = network.get_nodes_close_to(&node);

        for node0 in &close_group {
            for node1 in &close_group {
                if node0 == node1 { continue }
                assert!(network.is_node_connected_to(*node0, *node1));
                assert!(network.is_node_connected_to(*node1, *node0));
            }
        }
    }
}

#[test]
fn nodes_in_close_group_of_any_name_are_connected_to_each_other() {
    let network = create_network(NODES_COUNT);

    for _ in 0..SAMPLES {
        let name = rand::random();
        let close_group = network.get_nodes_close_to(&name);

        for node0 in &close_group {
            for node1 in &close_group {
                if node0 == node1 { continue }
                assert!(network.is_node_connected_to(*node0, *node1));
                assert!(network.is_node_connected_to(*node1, *node0));
            }
        }
    }
}

#[test]
fn messages_for_individual_nodes_reach_their_recipients() {
    let mut network = create_network(NODES_COUNT);

    for _ in 0..SAMPLES {
        let (node_a, node_b) = network.get_two_random_nodes();
        let node_a_name = network.get_node_name(node_a);
        let node_b_name = network.get_node_name(node_b);

        let message = Message::new(Destination::Node(node_a_name),
                                   Destination::Node(node_b_name));

        let message_id = message.id;

        network.send_message(node_a, message);
        assert!(network.has_node_message_in_inbox(node_b, message_id));
    }
}

#[test]
fn messages_for_groups_reach_all_members_of_the_recipient_group() {
    let mut network = create_network(NODES_COUNT);

    for _ in 0..SAMPLES {
        let sender = network.get_random_node();
        let sender_name = network.get_node_name(sender);

        let group_name = rand::random();
        let group_members = network.get_nodes_close_to(&group_name);

        let message = Message::new(Destination::Node(sender_name),
                                   Destination::Group(group_name));

        let message_id = message.id;

        network.send_message(sender, message);

        for node in group_members {
            assert!(network.has_node_message_in_inbox(node, message_id));
        }
    }
}

#[test]
fn only_original_sender_may_send_multiple_copies() {
    let mut network = create_network(NODES_COUNT);

    for _ in 0..SAMPLES {
        let (node_a, node_b) = network.get_two_random_nodes();
        let node_a_name = network.get_node_name(node_a);
        let node_b_name = network.get_node_name(node_b);

        let message = Message::new(Destination::Node(node_a_name),
                                   Destination::Node(node_b_name));

        let message_id = message.id;

        network.send_message(node_a, message);

        for node in network.get_all_nodes() {
            let count = network.get_message_stats(node).get_sent(message_id);

            if node != node_a {
                assert!(count <= 1);
            }
        }
    }
}

// TODO: this test is probably worthless, as it can only fail for extremely
// large networks or very unbalanced networks (the later may be possible to
// simulate though).
#[test]
fn maximum_number_of_contacts_in_routing_table() {
    let network = create_network(NODES_COUNT);

    for _ in 0..SAMPLES {
        let node = network.get_random_node();
        assert!(network.get_contact_count(node) <= 512 * GROUP_SIZE);
    }
}
