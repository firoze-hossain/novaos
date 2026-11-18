//! sdk/novaos-sdk/src/ipc.rs
//!
//! Inter-Process Communication (IPC) primitives for NovaOS applications.
//! Supports typed messages, asynchronous event subscription, and shared memory.

use std::collections::VecDeque;

/// Header metadata for an IPC message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageHeader {
    pub sender_pid: u32,
    pub receiver_pid: u32,
    pub message_type: u32,
    pub payload_len: u32,
}

/// A structured inter-process message with fixed maximum payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcMessage {
    pub header: MessageHeader,
    pub payload: Vec<u8>,
}

impl IpcMessage {
    pub fn new(sender_pid: u32, receiver_pid: u32, message_type: u32, payload: &[u8]) -> Self {
        Self {
            header: MessageHeader {
                sender_pid,
                receiver_pid,
                message_type,
                payload_len: payload.len() as u32,
            },
            payload: payload.to_vec(),
        }
    }

    pub fn text(sender_pid: u32, receiver_pid: u32, message_type: u32, text: &str) -> Self {
        Self::new(sender_pid, receiver_pid, message_type, text.as_bytes())
    }

    pub fn payload_as_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.payload)
    }
}

/// An IPC message channel for sending and receiving messages between processes.
#[derive(Debug)]
pub struct MessageChannel {
    pub local_pid: u32,
    inbox: VecDeque<IpcMessage>,
    outbox: VecDeque<IpcMessage>,
}

impl MessageChannel {
    pub fn new(local_pid: u32) -> Self {
        Self {
            local_pid,
            inbox: VecDeque::new(),
            outbox: VecDeque::new(),
        }
    }

    pub fn send(&mut self, receiver_pid: u32, message_type: u32, payload: &[u8]) -> Result<(), String> {
        let msg = IpcMessage::new(self.local_pid, receiver_pid, message_type, payload);
        self.outbox.push_back(msg);
        Ok(())
    }

    pub fn recv(&mut self) -> Option<IpcMessage> {
        self.inbox.pop_front()
    }

    pub fn inject_inbox(&mut self, msg: IpcMessage) {
        self.inbox.push_back(msg);
    }

    pub fn pending_outbox(&self) -> usize {
        self.outbox.len()
    }

    pub fn take_outbox(&mut self) -> Vec<IpcMessage> {
        self.outbox.drain(..).collect()
    }
}

/// Event bus for topic-based publish/subscribe notifications within or across apps.
pub struct EventBus {
    subscriptions: std::collections::HashMap<String, Vec<Box<dyn Fn(&[u8]) + Send + Sync>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            subscriptions: std::collections::HashMap::new(),
        }
    }

    pub fn subscribe<F>(&mut self, topic: &str, handler: F)
    where
        F: Fn(&[u8]) + Send + Sync + 'static,
    {
        self.subscriptions
            .entry(topic.to_string())
            .or_default()
            .push(Box::new(handler));
    }

    pub fn publish(&self, topic: &str, data: &[u8]) {
        if let Some(handlers) = self.subscriptions.get(topic) {
            for handler in handlers {
                handler(data);
            }
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared memory buffer channel for high-throughput zero-copy data exchange.
#[derive(Debug)]
pub struct ShmChannel {
    pub id: u32,
    pub size: usize,
    buffer: Vec<u8>,
}

impl ShmChannel {
    pub fn create(id: u32, size: usize) -> Self {
        Self {
            id,
            size,
            buffer: vec![0u8; size],
        }
    }

    pub fn write_at(&mut self, offset: usize, data: &[u8]) -> Result<(), String> {
        if offset + data.len() > self.size {
            return Err("Shared memory out of bounds write".to_string());
        }
        self.buffer[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }

    pub fn read_at(&self, offset: usize, len: usize) -> Result<&[u8], String> {
        if offset + len > self.size {
            return Err("Shared memory out of bounds read".to_string());
        }
        Ok(&self.buffer[offset..offset + len])
    }
}
