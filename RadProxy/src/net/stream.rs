//! Async TCP stream wrapper with optional `AES-128-CFB8` Minecraft encryption.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use rad_core::crypto::MinecraftCipher;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// Wraps a [`TcpStream`] with an optional [`MinecraftCipher`] so `copy_bidirectional`
/// and framed packet I/O work seamlessly before and after encryption is enabled.
pub struct EncryptedStream {
    inner: TcpStream,
    cipher: Option<MinecraftCipher>,
    enc_buf: Vec<u8>,
}

impl EncryptedStream {
    /// Creates a new unencrypted [`EncryptedStream`].
    #[must_use]
    pub fn new(inner: TcpStream) -> Self {
        Self {
            inner,
            cipher: None,
            enc_buf: Vec::new(),
        }
    }

    /// Enables `AES-128-CFB8` encryption and decryption for all subsequent I/O.
    pub fn enable_encryption(&mut self, shared_secret: &[u8; 16]) {
        self.cipher = Some(MinecraftCipher::new(shared_secret));
    }
}

impl AsyncRead for EncryptedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &poll {
            let after = buf.filled().len();
            if after > before
                && let Some(cipher) = self.cipher.as_mut()
            {
                cipher.decrypt(&mut buf.filled_mut()[before..after]);
            }
        }
        poll
    }
}

impl AsyncWrite for EncryptedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        if let Some(cipher) = this.cipher.as_mut() {
            let saved = cipher.clone();
            this.enc_buf.clear();
            this.enc_buf.extend_from_slice(buf);
            cipher.encrypt(&mut this.enc_buf);

            let poll = Pin::new(&mut this.inner).poll_write(cx, &this.enc_buf);
            match &poll {
                Poll::Ready(Ok(n)) if *n == buf.len() => {}
                Poll::Ready(Ok(n)) => {
                    *cipher = saved;
                    this.enc_buf[..*n].copy_from_slice(&buf[..*n]);
                    cipher.encrypt(&mut this.enc_buf[..*n]);
                }
                Poll::Pending | Poll::Ready(Err(_)) => {
                    *cipher = saved;
                }
            }
            poll
        } else {
            Pin::new(&mut this.inner).poll_write(cx, buf)
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        if self.cipher.is_none() {
            Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
        } else {
            let first = bufs
                .iter()
                .find(|b| !b.is_empty())
                .map_or(&[][..], |b| &**b);
            self.poll_write(cx, first)
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.cipher.is_none() && self.inner.is_write_vectored()
    }
}
